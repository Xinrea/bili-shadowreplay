use std::path::{Path, PathBuf};

use crate::constants;
use crate::progress::progress_reporter::{ProgressReporter, ProgressReporterTrait};
use crate::subtitle_generator::{GenerateResult, SubtitleGenerator, SubtitleGeneratorType};
use ffmpeg_utils::ffmpeg_command;

use super::{
    hwaccel,
    runner::{run_command, ProgressMode, TempArtifact},
};

/// Minimum output frame rate for rendered danmaku.
const MIN_DANMU_FPS: f64 = 60.0;

/// Raise low/unknown source rates without lowering sources already above the minimum.
fn danmu_output_fps(source_fps: f64) -> f64 {
    if source_fps.is_finite() && source_fps > 0.0 {
        source_fps.max(MIN_DANMU_FPS)
    } else {
        MIN_DANMU_FPS
    }
}

fn file_name_str(path: &Path) -> Result<&str, String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Invalid file name: {}", path.display()))
}

fn subtitle_filter_path(path: &Path) -> Result<String, String> {
    if cfg!(target_os = "windows") {
        Ok(format!(
            "'{}'",
            ffmpeg_utils::path_str(path)?
                .replace('\\', "\\\\")
                .replace(':', "\\:")
        ))
    } else {
        Ok(format!("'{}'", path.display()))
    }
}

/// Burn an SRT subtitle file into a video.
pub async fn encode_video_subtitle<R: ProgressReporterTrait>(
    reporter: &R,
    file: &Path,
    subtitle: &Path,
    srt_style: String,
) -> Result<String, String> {
    log::info!("Encode video subtitle task start: {}", file.display());
    log::info!("SRT style: {srt_style}");
    match std::fs::metadata(subtitle) {
        Ok(metadata) if metadata.len() == 0 => {
            return Err(format!("字幕文件为空：{}", subtitle.display()));
        }
        Ok(_) => {}
        Err(error) => return Err(format!("字幕文件不可用：{} ({error})", subtitle.display())),
    }

    let output_filename = format!("{}{}", constants::PREFIX_SUBTITLE, file_name_str(file)?);
    let output_path = file.with_file_name(&output_filename);
    let subtitle = subtitle_filter_path(subtitle)?;
    let vf = format!(
        "{},subtitles={subtitle}:force_style='{srt_style}'",
        hwaccel::H264_SCALE_PAD_FILTER
    );
    let mut command = ffmpeg_command();
    let video_encoder = hwaccel::get_x264_encoder().await;
    command.arg("-i").arg(file);
    hwaccel::apply_x264_encoder_args(&mut command, video_encoder, Some(&vf));
    command.args(["-c:a", "copy"]);
    hwaccel::apply_x264_quality_args(&mut command, video_encoder);
    command.args(["-y"]).arg(&output_path);

    run_command(
        command,
        ProgressMode::prefixed("压制中："),
        "Encode video subtitle",
        Some(reporter),
    )
    .await
    .map_err(|error| error.to_string())?;
    log::info!("Encode video subtitle task end: {}", output_path.display());
    Ok(output_filename)
}

/// Burn an ASS danmu file into a video.
pub async fn encode_video_danmu<R: ProgressReporterTrait>(
    reporter: Option<&R>,
    file: &Path,
    subtitle: &Path,
) -> Result<PathBuf, String> {
    log::info!("Encode video danmu task start: {}", file.display());
    let danmu_filename = format!("{}{}", constants::PREFIX_DANMAKU, file_name_str(file)?);
    let output_path = file.with_file_name(&danmu_filename);
    let subtitle = subtitle_filter_path(subtitle)?;
    let vf = format!("{},ass={subtitle}", hwaccel::H264_SCALE_PAD_FILTER);
    let source_fps = match ffmpeg_utils::extract_video_metadata(file).await {
        Ok(metadata) => metadata.fps,
        Err(error) => {
            log::warn!(
                "Failed to inspect source FPS for {}: {}; using {MIN_DANMU_FPS} FPS",
                file.display(),
                error
            );
            0.0
        }
    };
    let output_fps = danmu_output_fps(source_fps).to_string();

    let mut command = ffmpeg_command();
    let video_encoder = hwaccel::get_x264_encoder().await;
    command.arg("-i").arg(file);
    hwaccel::apply_x264_encoder_args(&mut command, video_encoder, Some(&vf));
    command.args(["-c:a", "copy"]);
    hwaccel::apply_x264_quality_args(&mut command, video_encoder);
    command.args(["-r", output_fps.as_str()]);
    command.args(["-y"]).arg(&output_path);

    run_command(
        command,
        ProgressMode::prefixed("压制中："),
        "Encode video danmu",
        reporter,
    )
    .await
    .map_err(|error| error.to_string())?;
    log::info!("Encode video danmu task end: {}", output_path.display());
    Ok(output_path)
}

/// Generate subtitles through one of the configured subtitle services.
#[allow(clippy::too_many_arguments)]
pub async fn generate_video_subtitle(
    reporter: Option<&ProgressReporter>,
    file: &Path,
    generator_type: SubtitleGeneratorType,
    resource_dir: &Path,
    whisper_model: &str,
    whisper_prompt: &str,
    openai_api_key: &str,
    openai_api_endpoint: &str,
    language_hint: &str,
) -> Result<GenerateResult, String> {
    match generator_type {
        SubtitleGeneratorType::Whisper => {
            if whisper_model.is_empty() {
                return Err("Whisper model not configured".to_string());
            }
            let vad_model = resource_dir.join("silero_vad.onnx");
            if !vad_model.is_file() {
                return Err(format!(
                    "Bundled Silero VAD model not found: {}",
                    vad_model.display()
                ));
            }
            let generator = crate::subtitle_generator::whisper_cpp::new(
                Path::new(whisper_model),
                whisper_prompt,
            )
            .await
            .map_err(|error| format!("Failed to initialize Whisper model: {error}"))?;

            if let Some(reporter) = reporter {
                reporter.update("提取完整音频中").await;
            }
            let full_wav = super::audio::extract_full_audio(file).await?;
            let full_wav_guard = TempArtifact::new(full_wav.clone());
            let audio = hound::WavReader::open(&full_wav).map_err(|error| error.to_string())?;
            let spec = audio.spec();
            let sample_rate = spec.sample_rate;
            let raw_samples: Vec<i16> = audio
                .into_samples::<i16>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("Failed to decode WAV samples: {error}"))?;
            let mut f32_samples = vec![0.0f32; raw_samples.len()];
            whisper_cpp_rs::convert_integer_to_float_audio(&raw_samples, &mut f32_samples)
                .map_err(|error| format!("Audio conversion error: {error}"))?;

            if let Some(reporter) = reporter {
                reporter.update("检测语音片段中").await;
            }
            let mut speech_segments =
                crate::audio_utils::silero_vad(&f32_samples, sample_rate, &vad_model)?;
            let audio_duration = f32_samples.len() as f64 / f64::from(sample_rate);
            if speech_segments.is_empty() && audio_duration > 0.0 {
                log::warn!("Silero VAD detected no speech; falling back to the full audio");
                speech_segments.push(crate::audio_utils::SpeechSegment {
                    start: 0.0,
                    end: audio_duration,
                });
            }
            let (energies, frame_sec) = crate::audio_utils::rms_energies(&f32_samples, sample_rate);
            log::info!(
                "Silero VAD detected {} speech segments from {:.1}s audio",
                speech_segments.len(),
                audio_duration
            );

            // Cut & Merge: normalize to ≤30s chunks
            let max_chunk = 30.0;
            let chunks = crate::audio_utils::cut_and_merge(
                &speech_segments,
                &energies,
                frame_sec,
                max_chunk, // cut_max: split segments >30s
                10.0,      // merge_max: merge adjacent segments ≤10s
            );
            log::info!(
                "Cut & Merge: {} speech segments → {} chunks (≤{}s each)",
                speech_segments.len(),
                chunks.len(),
                max_chunk
            );

            // Process each chunk
            let mut results = Vec::new();
            let mut rolling_context = String::new();
            let temp_dir = tempfile::tempdir().map_err(|error| error.to_string())?;
            let chunk_padding = 0.35;
            for (i, chunk) in chunks.iter().enumerate() {
                let segment_path = temp_dir.path().join(format!("seg_{i:03}.wav"));
                let padded_start = (chunk.start - chunk_padding).max(0.0);
                let padded_end = chunk.end + chunk_padding;
                let duration = padded_end - padded_start;
                if let Some(reporter) = reporter {
                    reporter
                        .update(&format!(
                            "字幕生成中 ({}/{}, {:.0}s-{:.0}s)",
                            i + 1,
                            chunks.len(),
                            padded_start,
                            padded_end
                        ))
                        .await;
                }
                match super::audio::extract_audio_segment(
                    file,
                    padded_start,
                    duration,
                    &segment_path,
                )
                .await
                {
                    Ok(()) => {
                        let chunk_generator = generator.with_previous_context(&rolling_context);
                        let result = chunk_generator
                            .generate_subtitle_with_confidence(
                                reporter,
                                &segment_path,
                                language_hint,
                            )
                            .await;
                        if let Ok(generated) = &result {
                            log::info!(
                                "Whisper chunk {} quality: confidence={:.3}, low_token_ratio={:.1}%, retried={}, eligible_for_prompt={}",
                                i,
                                generated.confidence.geometric_mean,
                                generated.confidence.low_token_ratio * 100.0,
                                generated.retried,
                                generated.eligible_for_prompt
                            );
                            if generated.eligible_for_prompt {
                                rolling_context =
                                    crate::subtitle_generator::whisper_cpp::update_rolling_context(
                                        &rolling_context,
                                        &generated.result,
                                    );
                                log::debug!(
                                    "Whisper rolling context updated to {} characters",
                                    rolling_context.chars().count()
                                );
                            } else {
                                log::warn!(
                                    "Whisper chunk {} excluded from rolling prompt: confidence={:.3}, low_token_ratio={:.1}%",
                                    i,
                                    generated.confidence.geometric_mean,
                                    generated.confidence.low_token_ratio * 100.0
                                );
                            }
                        }
                        results.push((
                            (padded_start * 1000.0).round() as u64,
                            result.map(|generated| generated.result),
                        ));
                    }
                    Err(error) => {
                        log::error!("Failed to extract segment {i}: {error}");
                        continue;
                    }
                }
            }
            // The chunk directory and the full-audio WAV are removed by their
            // guards, including on the early returns above.
            drop(temp_dir);
            drop(full_wav_guard);

            let mut full_result = GenerateResult {
                subtitle_id: String::new(),
                subtitle_content: vec![],
                generator_type: SubtitleGeneratorType::Whisper,
            };
            for (offset_ms, result) in &results {
                if let Ok(result) = result {
                    full_result.subtitle_id = result.subtitle_id.clone();
                    full_result.concat_with_offset_ms(result, *offset_ms);
                }
            }
            full_result.clamp_overlaps();
            Ok(full_result)
        }
        SubtitleGeneratorType::WhisperOnline => {
            if openai_api_key.is_empty() {
                return Err("API key not configured".to_string());
            }
            let generator = crate::subtitle_generator::whisper_online::new(
                Some(openai_api_endpoint),
                Some(openai_api_key),
                Some(whisper_prompt),
            )
            .await
            .map_err(|_| "Failed to initialize Whisper Online".to_string())?;
            let chunk_dir = super::audio::extract_audio_chunks_guarded(file, "wav").await?;
            let mut chunk_paths = std::fs::read_dir(chunk_dir.path())
                .map_err(|error| format!("Failed to read chunk directory: {error}"))?
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("Failed to read directory entry: {error}"))?;
            chunk_paths.sort_by_key(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });

            let mut full_result = GenerateResult {
                subtitle_id: String::new(),
                subtitle_content: vec![],
                generator_type: SubtitleGeneratorType::WhisperOnline,
            };
            for (i, path) in chunk_paths.iter().enumerate() {
                if let Ok(result) = generator
                    .generate_subtitle(reporter, path, language_hint)
                    .await
                {
                    full_result.subtitle_id = result.subtitle_id.clone();
                    full_result.concat(&result, 30 * i as u64);
                }
            }
            Ok(full_result)
        }
        SubtitleGeneratorType::PowerLive => {
            let generator = crate::subtitle_generator::powerlive::new(
                "pk_d2755cd38ef03f7ed3a92be1f1471e4adea90a1a5d4b3900345298a68fba0821",
            )
            .await
            .map_err(|_| "Failed to initialize PowerLive".to_string())?;
            let extension = file
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or_default();
            let audio_file = if matches!(extension, "opus" | "wav" | "mp3" | "m4a" | "flac") {
                file.to_path_buf()
            } else {
                file.with_extension("opus")
            };
            if !audio_file.exists() {
                return Err("Audio file not found".to_string());
            }
            generator
                .generate_subtitle(reporter, &audio_file, language_hint)
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{danmu_output_fps, MIN_DANMU_FPS};

    #[test]
    fn danmu_output_fps_applies_the_minimum_without_clamping_higher_sources() {
        assert_eq!(danmu_output_fps(30.0), MIN_DANMU_FPS);
        assert_eq!(danmu_output_fps(59.94), MIN_DANMU_FPS);
        assert_eq!(danmu_output_fps(60.0), MIN_DANMU_FPS);
        assert_eq!(danmu_output_fps(120.0), 120.0);
    }

    #[test]
    fn danmu_output_fps_falls_back_to_the_minimum_for_unknown_sources() {
        assert_eq!(danmu_output_fps(0.0), MIN_DANMU_FPS);
        assert_eq!(danmu_output_fps(f64::NAN), MIN_DANMU_FPS);
        assert_eq!(danmu_output_fps(f64::INFINITY), MIN_DANMU_FPS);
    }
}
