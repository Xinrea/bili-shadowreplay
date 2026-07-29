use async_trait::async_trait;

use crate::{
    progress::progress_reporter::ProgressReporterTrait,
    subtitle_generator::{GenerateResult, SubtitleGeneratorType},
};
use async_std::sync::{Arc, RwLock};
use std::path::Path;
use whisper_cpp_rs::{FullParams, SamplingStrategy, WhisperContext};

use super::SubtitleGenerator;

const ROLLING_CONTEXT_MAX_CHARS: usize = 200;
const SUBTITLE_MAX_CHARS: usize = 15;
const INITIAL_BEAM_SIZE: i32 = 5;
const RETRY_BEAM_SIZE: i32 = 8;
// whisper.cpp's upstream default. Patience is currently not implemented there.
const BEAM_SEARCH_PATIENCE_DEFAULT: f32 = -1.0;
const LOW_TOKEN_PROBABILITY: f32 = 0.2;
const MIN_GEOMETRIC_CONFIDENCE: f32 = 0.35;
const MAX_LOW_TOKEN_RATIO: f32 = 0.35;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConfidenceMetrics {
    pub geometric_mean: f32,
    pub low_token_ratio: f32,
    pub token_count: usize,
}

impl ConfidenceMetrics {
    fn is_low_confidence(&self) -> bool {
        self.token_count == 0
            || self.geometric_mean < MIN_GEOMETRIC_CONFIDENCE
            || self.low_token_ratio > MAX_LOW_TOKEN_RATIO
    }

    fn is_better_than(&self, other: &Self) -> bool {
        const EPSILON: f32 = 0.01;
        self.geometric_mean > other.geometric_mean + EPSILON
            || ((self.geometric_mean - other.geometric_mean).abs() <= EPSILON
                && self.low_token_ratio < other.low_token_ratio)
    }
}

pub struct WhisperGenerateResult {
    pub result: GenerateResult,
    pub confidence: ConfidenceMetrics,
    pub retried: bool,
    pub eligible_for_prompt: bool,
}

struct TranscriptionCandidate {
    result: GenerateResult,
    confidence: ConfidenceMetrics,
}

fn is_confidence_token(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && !text.starts_with("<|")
        && !text.starts_with("[_")
        && text.chars().any(char::is_alphanumeric)
}

fn confidence_from_probabilities(
    probabilities: impl IntoIterator<Item = f32>,
) -> ConfidenceMetrics {
    let mut log_probability_sum = 0.0f32;
    let mut low_token_count = 0usize;
    let mut token_count = 0usize;

    for probability in probabilities {
        let probability = if probability.is_finite() {
            probability.clamp(1e-6, 1.0)
        } else {
            1e-6
        };
        log_probability_sum += probability.ln();
        low_token_count += usize::from(probability < LOW_TOKEN_PROBABILITY);
        token_count += 1;
    }

    if token_count == 0 {
        return ConfidenceMetrics {
            geometric_mean: 0.0,
            low_token_ratio: 1.0,
            token_count: 0,
        };
    }

    ConfidenceMetrics {
        geometric_mean: (log_probability_sum / token_count as f32).exp(),
        low_token_ratio: low_token_count as f32 / token_count as f32,
        token_count,
    }
}

fn split_subtitle_text(text: &str, max_chars: usize) -> Vec<String> {
    let chars: Vec<char> = text.trim().chars().collect();
    if chars.is_empty() || max_chars == 0 {
        return Vec::new();
    }

    let mut parts = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let hard_end = (start + max_chars).min(chars.len());
        let mut end = hard_end;

        if hard_end < chars.len() {
            let range = &chars[start..hard_end];
            let strong_boundary = range
                .iter()
                .rposition(|c| matches!(c, '。' | '！' | '？' | '!' | '?' | '；' | ';'))
                .map(|index| start + index + 1);
            let soft_min = max_chars / 2;
            let soft_boundary = range
                .iter()
                .enumerate()
                .filter(|(index, c)| {
                    *index + 1 >= soft_min && (matches!(c, '，' | ',' | '、') || c.is_whitespace())
                })
                .map(|(index, _)| start + index + 1)
                .next_back();
            end = strong_boundary.or(soft_boundary).unwrap_or(hard_end);
        }

        let part: String = chars[start..end].iter().collect();
        let part = part.trim();
        if !part.is_empty() {
            parts.push(part.to_string());
        }
        start = end;
        while start < chars.len() && chars[start].is_whitespace() {
            start += 1;
        }
    }

    parts
}

fn time_from_millis(total_ms: u64) -> srtparse::Time {
    srtparse::Time {
        hours: total_ms / 3_600_000,
        minutes: (total_ms / 60_000) % 60,
        seconds: (total_ms / 1_000) % 60,
        milliseconds: total_ms % 1_000,
    }
}

fn audio_duration_timestamp(sample_count: usize) -> i64 {
    // whisper.cpp timestamps use 10 ms units. Round to the nearest unit.
    ((sample_count as u64 * 100 + 8_000) / 16_000) as i64
}

fn clamp_segment_timestamps(
    start_timestamp: i64,
    end_timestamp: i64,
    audio_end_timestamp: i64,
) -> Option<(i64, i64)> {
    let start_timestamp = start_timestamp.clamp(0, audio_end_timestamp);
    let end_timestamp = end_timestamp.clamp(start_timestamp, audio_end_timestamp);
    (end_timestamp > start_timestamp).then_some((start_timestamp, end_timestamp))
}

fn subtitle_items_for_segment(
    text: &str,
    start_timestamp: i64,
    end_timestamp: i64,
    first_pos: usize,
) -> Vec<srtparse::Item> {
    let parts = split_subtitle_text(text, SUBTITLE_MAX_CHARS);
    if parts.is_empty() {
        return Vec::new();
    }

    // whisper.cpp timestamps are in 10 ms units. Without reliable token
    // timestamps, distribute post-processed lines proportionally by length.
    let start_ms = start_timestamp.max(0) as u64 * 10;
    let end_ms = end_timestamp.max(start_timestamp) as u64 * 10;
    let duration_ms = end_ms.saturating_sub(start_ms);
    if parts.len() > 1 && duration_ms < parts.len() as u64 {
        return vec![srtparse::Item {
            pos: first_pos,
            start_time: time_from_millis(start_ms),
            end_time: time_from_millis(end_ms),
            text: text.trim().to_string(),
        }];
    }

    let weights: Vec<u64> = parts
        .iter()
        .map(|part| part.chars().count().max(1) as u64)
        .collect();
    let total_weight: u64 = weights.iter().sum();
    let mut consumed_weight = 0;
    let mut line_start_ms = start_ms;

    parts
        .into_iter()
        .zip(weights)
        .enumerate()
        .map(|(index, (part, weight))| {
            consumed_weight += weight;
            let line_end_ms = if consumed_weight == total_weight {
                end_ms
            } else {
                start_ms + duration_ms * consumed_weight / total_weight
            };
            let item = srtparse::Item {
                pos: first_pos + index,
                start_time: time_from_millis(line_start_ms),
                end_time: time_from_millis(line_end_ms),
                text: part,
            };
            line_start_ms = line_end_ms;
            item
        })
        .collect()
}

fn take_tail_chars(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    value
        .chars()
        .skip(char_count.saturating_sub(max_chars))
        .collect()
}

fn compose_prompt(base_prompt: &str, previous_context: &str) -> String {
    match (base_prompt.trim(), previous_context.trim()) {
        ("", "") => String::new(),
        (base, "") => base.to_string(),
        ("", context) => context.to_string(),
        (base, context) => format!("{base}\n{context}"),
    }
}

pub fn update_rolling_context(previous_context: &str, result: &GenerateResult) -> String {
    let current_text = result
        .subtitle_content
        .iter()
        .map(|item| item.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if current_text.is_empty() {
        return previous_context.to_string();
    }

    let combined = if previous_context.trim().is_empty() {
        current_text
    } else {
        format!("{} {}", previous_context.trim(), current_text)
    };
    take_tail_chars(&combined, ROLLING_CONTEXT_MAX_CHARS)
}

fn pcm_i16_to_mono_f32(samples: &[i16], channels: u16) -> Result<Vec<f32>, String> {
    if channels == 0 {
        return Err("WAV channel count must be greater than zero".to_string());
    }

    let channels = channels as usize;
    if !samples.len().is_multiple_of(channels) {
        return Err(format!(
            "WAV sample count {} is not divisible by channel count {channels}",
            samples.len()
        ));
    }

    let mut mono = Vec::with_capacity(samples.len() / channels);
    for frame in samples.chunks_exact(channels) {
        let sum: f32 = frame.iter().map(|&sample| sample as f32 / 32768.0).sum();
        mono.push(sum / channels as f32);
    }
    Ok(mono)
}

#[derive(Clone)]
pub struct WhisperCPP {
    ctx: Arc<RwLock<WhisperContext>>,
    base_prompt: String,
    previous_context: String,
}

pub async fn new(model: &Path, prompt: &str) -> Result<WhisperCPP, String> {
    let model_path = model.to_string_lossy();
    let ctx = WhisperContext::new(&model_path).map_err(|e| {
        log::error!("Create whisper context failed: {e}");
        e.to_string()
    })?;

    Ok(WhisperCPP {
        ctx: Arc::new(RwLock::new(ctx)),
        base_prompt: prompt.to_string(),
        previous_context: String::new(),
    })
}

impl WhisperCPP {
    pub fn with_previous_context(&self, previous_context: &str) -> Self {
        Self {
            ctx: self.ctx.clone(),
            base_prompt: self.base_prompt.clone(),
            previous_context: previous_context.to_string(),
        }
    }

    async fn transcribe_samples(
        &self,
        samples: &[f32],
        language_hint: &str,
        beam_size: i32,
        include_previous_context: bool,
    ) -> Result<TranscriptionCandidate, String> {
        let ctx = self.ctx.read().await;
        let mut state = ctx.create_state().map_err(|e| e.to_string())?;
        let mut params = FullParams::new(SamplingStrategy::BeamSearch {
            beam_size,
            patience: BEAM_SEARCH_PATIENCE_DEFAULT,
        });

        params.set_language(Some(language_hint));
        let prompt = if include_previous_context {
            compose_prompt(&self.base_prompt, &self.previous_context)
        } else {
            self.base_prompt.trim().to_string()
        };
        params.set_initial_prompt(&prompt);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        if let Err(e) = state.full(&ctx, &params, samples) {
            log::error!("failed to run model: {e}");
            return Err(e.to_string());
        }

        let num_segments = state.full_n_segments();
        let mut subtitle_content = Vec::new();
        let mut probabilities = Vec::new();
        let audio_end_timestamp = audio_duration_timestamp(samples.len());

        for i in 0..num_segments {
            let segment_text = state.get_segment_text(i).unwrap_or_default();
            if segment_text.trim().is_empty() {
                continue;
            }

            let raw_start_timestamp = state.get_segment_t0(i);
            let raw_end_timestamp = state.get_segment_t1(i);
            let Some((start_timestamp, end_timestamp)) = clamp_segment_timestamps(
                raw_start_timestamp,
                raw_end_timestamp,
                audio_end_timestamp,
            ) else {
                log::warn!(
                    "Discarding Whisper segment outside audio duration: segment={} raw={}..{}, audio_end={}",
                    i,
                    raw_start_timestamp,
                    raw_end_timestamp,
                    audio_end_timestamp
                );
                continue;
            };
            if start_timestamp != raw_start_timestamp || end_timestamp != raw_end_timestamp {
                log::warn!(
                    "Clamped Whisper segment timestamps to audio duration: segment={} raw={}..{}, clamped={}..{}, audio_end={}",
                    i,
                    raw_start_timestamp,
                    raw_end_timestamp,
                    start_timestamp,
                    end_timestamp,
                    audio_end_timestamp
                );
            }

            probabilities.extend(
                state
                    .get_segment_tokens(&ctx, i)
                    .into_iter()
                    .filter(|token| is_confidence_token(&token.text))
                    .map(|token| token.p),
            );

            let first_pos = subtitle_content.len() + 1;
            subtitle_content.extend(subtitle_items_for_segment(
                &segment_text,
                start_timestamp,
                end_timestamp,
                first_pos,
            ));
        }

        Ok(TranscriptionCandidate {
            result: GenerateResult {
                generator_type: SubtitleGeneratorType::Whisper,
                subtitle_id: String::new(),
                subtitle_content,
            },
            confidence: confidence_from_probabilities(probabilities),
        })
    }

    pub async fn generate_subtitle_with_confidence(
        &self,
        reporter: Option<&(impl ProgressReporterTrait + 'static)>,
        audio_path: &Path,
        language_hint: &str,
    ) -> Result<WhisperGenerateResult, String> {
        log::info!("Generating subtitle for {:?}", audio_path);
        let start_time = std::time::Instant::now();
        let mut audio = hound::WavReader::open(audio_path).map_err(|e| e.to_string())?;
        let spec = audio.spec();
        if spec.sample_rate != 16_000 {
            return Err(format!(
                "Whisper expects 16000 Hz audio, got {} Hz",
                spec.sample_rate
            ));
        }
        let samples: Vec<i16> = audio
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to decode WAV samples: {e}"))?;

        if let Some(reporter) = reporter {
            reporter.update("处理音频中").await;
        }
        let samples = pcm_i16_to_mono_f32(&samples, spec.channels)?;

        if let Some(reporter) = reporter {
            reporter.update("生成字幕中").await;
        }
        let initial = self
            .transcribe_samples(&samples, language_hint, INITIAL_BEAM_SIZE, true)
            .await?;
        log::info!(
            "Whisper confidence: geometric_mean={:.3}, low_token_ratio={:.1}%, tokens={}, retry={}",
            initial.confidence.geometric_mean,
            initial.confidence.low_token_ratio * 100.0,
            initial.confidence.token_count,
            initial.confidence.is_low_confidence()
        );

        let (selected, retried) = if initial.confidence.is_low_confidence() {
            if let Some(reporter) = reporter {
                reporter.update("低置信度片段重试中").await;
            }
            let retry = self
                .transcribe_samples(&samples, language_hint, RETRY_BEAM_SIZE, false)
                .await?;
            let use_retry = retry.confidence.is_better_than(&initial.confidence);
            log::info!(
                "Whisper retry confidence: geometric_mean={:.3}, low_token_ratio={:.1}%, tokens={}, selected={}",
                retry.confidence.geometric_mean,
                retry.confidence.low_token_ratio * 100.0,
                retry.confidence.token_count,
                if use_retry { "retry" } else { "initial" }
            );
            (if use_retry { retry } else { initial }, true)
        } else {
            (initial, false)
        };
        let eligible_for_prompt = !selected.confidence.is_low_confidence();

        log::info!(
            "Whisper result: confidence={:.3}, low_token_ratio={:.1}%, retried={}, eligible_for_prompt={}, elapsed={:.2}s",
            selected.confidence.geometric_mean,
            selected.confidence.low_token_ratio * 100.0,
            retried,
            eligible_for_prompt,
            start_time.elapsed().as_secs_f64()
        );

        Ok(WhisperGenerateResult {
            result: selected.result,
            confidence: selected.confidence,
            retried,
            eligible_for_prompt,
        })
    }
}

#[async_trait]
impl SubtitleGenerator for WhisperCPP {
    async fn generate_subtitle(
        &self,
        reporter: Option<&(impl ProgressReporterTrait + 'static)>,
        audio_path: &Path,
        language_hint: &str,
    ) -> Result<GenerateResult, String> {
        self.generate_subtitle_with_confidence(reporter, audio_path, language_hint)
            .await
            .map(|generated| generated.result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_prompt_appends_previous_context() {
        assert_eq!(
            compose_prompt("中文游戏直播", "上一句字幕"),
            "中文游戏直播\n上一句字幕"
        );
        assert_eq!(compose_prompt("", "上一句字幕"), "上一句字幕");
        assert_eq!(compose_prompt("中文游戏直播", ""), "中文游戏直播");
    }

    #[test]
    fn rolling_context_keeps_only_the_tail() {
        let result = GenerateResult {
            generator_type: SubtitleGeneratorType::Whisper,
            subtitle_id: String::new(),
            subtitle_content: vec![srtparse::Item {
                pos: 1,
                start_time: srtparse::Time {
                    hours: 0,
                    minutes: 0,
                    seconds: 0,
                    milliseconds: 0,
                },
                end_time: srtparse::Time {
                    hours: 0,
                    minutes: 0,
                    seconds: 1,
                    milliseconds: 0,
                },
                text: "新".repeat(100),
            }],
        };
        let context = update_rolling_context(&"旧".repeat(150), &result);
        assert_eq!(context.chars().count(), ROLLING_CONTEXT_MAX_CHARS);
        assert!(context.ends_with(&"新".repeat(100)));
    }

    #[test]
    fn confidence_uses_geometric_mean_and_low_token_ratio() {
        let confidence = confidence_from_probabilities([0.8, 0.6, 0.1]);
        assert_eq!(confidence.token_count, 3);
        assert!((confidence.geometric_mean - 0.363).abs() < 0.001);
        assert!((confidence.low_token_ratio - 1.0 / 3.0).abs() < 0.001);
        assert!(!confidence.is_low_confidence());

        let confidence = confidence_from_probabilities([0.8, 0.1, 0.1]);
        assert!(confidence.is_low_confidence());
    }

    #[test]
    fn confidence_ignores_special_and_punctuation_only_tokens() {
        assert!(!is_confidence_token("<|endoftext|>"));
        assert!(!is_confidence_token("[_BEG_]"));
        assert!(!is_confidence_token("。！"));
        assert!(is_confidence_token(" 中文"));
        assert!(is_confidence_token(" word"));
    }

    #[test]
    fn subtitle_text_is_split_after_recognition() {
        let parts = split_subtitle_text("这是第一句。这是一句需要按照逗号，继续拆分的字幕。", 15);
        assert_eq!(
            parts,
            vec!["这是第一句。", "这是一句需要按照逗号，", "继续拆分的字幕。"]
        );
        assert!(parts.iter().all(|part| part.chars().count() <= 15));

        assert_eq!(
            split_subtitle_text("this is a subtitle line for testing", 15),
            vec!["this is a", "subtitle line", "for testing"]
        );
    }

    #[test]
    fn post_processed_lines_share_original_segment_time() {
        let items = subtitle_items_for_segment(
            "这是一句比较长的字幕，需要在识别完成后再拆分。",
            100,
            500,
            3,
        );
        assert!(items.len() > 1);
        assert_eq!(items[0].pos, 3);
        assert_eq!(items[0].start_time.into_duration().as_millis(), 1_000);
        assert_eq!(
            items.last().unwrap().end_time.into_duration().as_millis(),
            5_000
        );
        assert!(items
            .windows(2)
            .all(|pair| pair[0].end_time == pair[1].start_time));
    }

    #[test]
    fn whisper_timestamps_are_clamped_to_the_real_audio_duration() {
        assert_eq!(audio_duration_timestamp(16_000 * 8), 800);
        assert_eq!(clamp_segment_timestamps(0, 3_000, 800), Some((0, 800)));
        assert_eq!(clamp_segment_timestamps(900, 3_000, 800), None);
    }

    /// Run whisper on test audio and validate output.
    async fn run_whisper_test() -> Vec<srtparse::Item> {
        let model_path = Path::new("tests/model/ggml-tiny-q5_1.bin");
        let audio_path = Path::new("tests/audio/test.wav");
        assert!(model_path.exists(), "Model not found");
        assert!(audio_path.exists(), "Test audio not found");

        let whisper = new(model_path, "").await.expect("Failed to create whisper");
        let audio = hound::WavReader::open(audio_path).unwrap();
        let audio_samples: Vec<i16> = audio.into_samples::<i16>().map(|x| x.unwrap()).collect();
        let mut state = whisper
            .ctx
            .read()
            .await
            .create_state()
            .expect("Failed to create state");

        let mut params = FullParams::new(SamplingStrategy::BeamSearch {
            beam_size: 5,
            patience: BEAM_SEARCH_PATIENCE_DEFAULT,
        });
        params.set_language(Some("auto"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        let mut inter = vec![0.0f32; audio_samples.len()];
        whisper_cpp_rs::convert_integer_to_float_audio(&audio_samples, &mut inter).unwrap();
        let mut mono = vec![0.0f32; audio_samples.len() / 2];
        whisper_cpp_rs::convert_stereo_to_mono_audio(&inter, &mut mono).unwrap();
        state
            .full(&*whisper.ctx.read().await, &params, &mono)
            .unwrap();

        let num_segments = state.full_n_segments();
        let mut subtitle = Vec::new();
        for i in 0..num_segments {
            let text = state.get_segment_text(i).unwrap_or_default();
            if text.trim().is_empty() {
                continue;
            }
            let t0 = state.get_segment_t0(i);
            let t1 = state.get_segment_t1(i);
            let first_pos = subtitle.len() + 1;
            subtitle.extend(subtitle_items_for_segment(&text, t0, t1, first_pos));
        }
        subtitle
    }

    #[tokio::test]
    #[ignore = "Requires whisper model and test audio"]
    async fn test_generate_subtitle_with_real_audio() {
        let items = run_whisper_test().await;
        println!("Generated {} subtitle items", items.len());
        assert!(!items.is_empty(), "Subtitle content must not be empty");

        for item in &items {
            assert!(
                !item.text.trim().is_empty(),
                "Item {} has empty text",
                item.pos
            );

            let st = item.start_time.hours * 3600
                + item.start_time.minutes * 60
                + item.start_time.seconds;
            let et =
                item.end_time.hours * 3600 + item.end_time.minutes * 60 + item.end_time.seconds;
            assert!(
                et > st || (et == st && item.end_time.milliseconds > item.start_time.milliseconds),
                "Item {} end <= start",
                item.pos
            );
            assert!(
                !item.text.contains("[_"),
                "Item {} has special token",
                item.pos
            );
            assert!(
                !item.text.contains("<|"),
                "Item {} has special token",
                item.pos
            );

            if item.pos <= 8 {
                println!(
                    "  #{}: {:02}:{:02}:{:02},{:03} --> {:02}:{:02}:{:02},{:03} |{}|",
                    item.pos,
                    item.start_time.hours,
                    item.start_time.minutes,
                    item.start_time.seconds,
                    item.start_time.milliseconds,
                    item.end_time.hours,
                    item.end_time.minutes,
                    item.end_time.seconds,
                    item.end_time.milliseconds,
                    item.text
                );
            }
        }
        println!("All validations passed.");
    }

    #[test]
    fn test_pcm_i16_mono_is_not_shortened() {
        let mono = pcm_i16_to_mono_f32(&[32767, 0, -32768], 1).unwrap();
        assert_eq!(mono.len(), 3);
        assert!(mono[0] > 0.99);
        assert_eq!(mono[1], 0.0);
        assert_eq!(mono[2], -1.0);
    }

    #[test]
    fn test_pcm_i16_stereo_is_averaged_per_frame() {
        let mono = pcm_i16_to_mono_f32(&[32767, -32768, 16384, 16384], 2).unwrap();
        assert_eq!(mono.len(), 2);
        assert!(mono[0].abs() < 0.001);
        assert!((mono[1] - 0.5).abs() < 0.001);
    }
}
