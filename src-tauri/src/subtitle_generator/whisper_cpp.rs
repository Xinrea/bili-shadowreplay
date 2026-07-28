use async_trait::async_trait;

use crate::{
    progress::progress_reporter::ProgressReporterTrait,
    subtitle_generator::{GenerateResult, SubtitleGeneratorType},
};
use async_std::sync::{Arc, RwLock};
use std::path::Path;
use whisper_cpp_rs::{FullParams, SamplingStrategy, WhisperContext};

use super::SubtitleGenerator;

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
    prompt: String,
}

pub async fn new(model: &Path, prompt: &str) -> Result<WhisperCPP, String> {
    let model_path = model.to_string_lossy();
    let ctx = WhisperContext::new(&model_path).map_err(|e| {
        log::error!("Create whisper context failed: {e}");
        e.to_string()
    })?;

    Ok(WhisperCPP {
        ctx: Arc::new(RwLock::new(ctx)),
        prompt: prompt.to_string(),
    })
}

#[async_trait]
impl SubtitleGenerator for WhisperCPP {
    async fn generate_subtitle(
        &self,
        reporter: Option<&(impl ProgressReporterTrait + 'static)>,
        audio_path: &Path,
        language_hint: &str,
    ) -> Result<GenerateResult, String> {
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

        let state = self.ctx.read().await.create_state();
        if let Err(e) = state {
            return Err(e.to_string());
        }

        let mut state = state.unwrap();

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 5 });

        // and set the language
        params.set_language(Some(language_hint));
        params.set_initial_prompt(self.prompt.as_str());

        // we also explicitly disable anything that prints to stdout
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        // NOTE: token_timestamps is not compatible with GGML format
        // models — the token data contains garbage timestamps that
        // produce invalid SRT output.  Use max_len instead for finer
        // segmentation (whisper splits at word boundaries internally).
        // params.set_token_timestamps(true);
        params.set_max_len(15);

        if let Some(reporter) = reporter {
            reporter.update("处理音频中").await;
        }
        let samples = pcm_i16_to_mono_f32(&samples, spec.channels)?;

        if let Some(reporter) = reporter {
            reporter.update("生成字幕中").await;
        }
        if let Err(e) = state.full(&*self.ctx.read().await, &params, &samples[..]) {
            log::error!("failed to run model: {e}");
            return Err(e.to_string());
        }

        // Fetch results using whisper's built-in segment-level timestamps.
        // With max_len=15, whisper internally splits at word boundaries,
        // producing more segments with tighter timestamps.
        let num_segments = state.full_n_segments();
        let mut subtitle = String::new();

        let format_time = |timestamp: f64| {
            let hours = (timestamp / 3600.0).floor();
            let minutes = ((timestamp - hours * 3600.0) / 60.0).floor();
            let seconds = (timestamp - hours * 3600.0 - minutes * 60.0).floor();
            let milliseconds =
                ((timestamp - hours * 3600.0 - minutes * 60.0 - seconds) * 1000.0).floor() as u32;
            format!("{hours:02}:{minutes:02}:{seconds:02},{milliseconds:03}")
        };

        for i in 0..num_segments {
            let segment_text = state.get_segment_text(i).unwrap_or_default();
            if segment_text.trim().is_empty() {
                continue;
            }

            let start_timestamp = state.get_segment_t0(i);
            let end_timestamp = state.get_segment_t1(i);

            let line = format!(
                "{}\n{} --> {}\n{}\n\n",
                i + 1,
                format_time(start_timestamp as f64 / 100.0),
                format_time(end_timestamp as f64 / 100.0),
                segment_text.trim(),
            );

            subtitle.push_str(&line);
        }

        log::info!("Time taken: {} seconds", start_time.elapsed().as_secs_f64());

        let subtitle_content =
            srtparse::from_str(&subtitle).map_err(|e| format!("Failed to parse subtitle: {e}"))?;

        Ok(GenerateResult {
            generator_type: SubtitleGeneratorType::Whisper,
            subtitle_id: String::new(),
            subtitle_content,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run whisper on test audio and validate output.
    async fn run_whisper_test(max_len: i32) -> Vec<srtparse::Item> {
        let model_path = Path::new("tests/model/ggml-tiny-q5_1.bin");
        let audio_path = Path::new("tests/audio/test.wav");
        assert!(model_path.exists(), "Model not found");
        assert!(audio_path.exists(), "Test audio not found");

        // Build params manually to control max_len
        let whisper = new(model_path, "").await.expect("Failed to create whisper");
        let audio = hound::WavReader::open(audio_path).unwrap();
        let audio_samples: Vec<i16> = audio.into_samples::<i16>().map(|x| x.unwrap()).collect();
        let mut state = whisper
            .ctx
            .read()
            .await
            .create_state()
            .expect("Failed to create state");

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 5 });
        params.set_language(Some("auto"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_max_len(max_len);

        let mut inter = vec![0.0f32; audio_samples.len()];
        whisper_cpp_rs::convert_integer_to_float_audio(&audio_samples, &mut inter).unwrap();
        let mut mono = vec![0.0f32; audio_samples.len() / 2];
        whisper_cpp_rs::convert_stereo_to_mono_audio(&inter, &mut mono).unwrap();
        state
            .full(&*whisper.ctx.read().await, &params, &mono)
            .unwrap();

        let num_segments = state.full_n_segments();
        let mut subtitle = String::new();
        let format_time = |ts: f64| {
            let h = (ts / 3600.0).floor();
            let m = ((ts - h * 3600.0) / 60.0).floor();
            let s = (ts - h * 3600.0 - m * 60.0).floor();
            let ms = ((ts - h * 3600.0 - m * 60.0 - s) * 1000.0).floor() as u32;
            format!("{h:02}:{m:02}:{s:02},{ms:03}")
        };
        for i in 0..num_segments {
            let text = state.get_segment_text(i).unwrap_or_default();
            if text.trim().is_empty() {
                continue;
            }
            let t0 = state.get_segment_t0(i);
            let t1 = state.get_segment_t1(i);
            subtitle.push_str(&format!(
                "{}\n{} --> {}\n{}\n\n",
                i + 1,
                format_time(t0 as f64 / 100.0),
                format_time(t1 as f64 / 100.0),
                text.trim()
            ));
        }
        srtparse::from_str(&subtitle).unwrap()
    }

    #[tokio::test]
    #[ignore = "Requires whisper model and test audio"]
    async fn test_parameter_sweep() {
        for max_len in [0, 5, 10, 15, 30] {
            let items = run_whisper_test(max_len).await;
            let avg_dur = if items.is_empty() {
                0.0
            } else {
                let total: u64 = items
                    .iter()
                    .map(|i| {
                        (i.end_time.hours * 3600 + i.end_time.minutes * 60 + i.end_time.seconds)
                            - (i.start_time.hours * 3600
                                + i.start_time.minutes * 60
                                + i.start_time.seconds)
                    })
                    .sum();
                total as f64 / items.len() as f64
            };
            println!(
                "max_len={:>3}: {:>3} items, avg {:>5.1}s/item, first='{}'",
                max_len,
                items.len(),
                avg_dur,
                items.first().map(|i| i.text.as_str()).unwrap_or("")
            );
        }
    }

    #[tokio::test]
    #[ignore = "Requires whisper model and test audio"]
    async fn test_generate_subtitle_with_real_audio() {
        let items = run_whisper_test(15).await;
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
