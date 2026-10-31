use std::path::Path;

use crate::constants;
use crate::progress::progress_reporter::{ProgressReporter, ProgressReporterTrait};
use crate::subtitle_generator::{GenerateResult, SubtitleGenerator, SubtitleGeneratorType};
#[cfg(test)]
use ffmpeg_utils::ffmpeg_command;

use super::{
    hwaccel,
    runner::{ProgressMode, ProgressTotal, TempArtifact},
};

fn file_name_str(path: &Path) -> Result<&str, String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Invalid file name: {}", path.display()))
}

/// Burn SRT with the native Skia subtitle renderer.
#[cfg(test)]
pub async fn encode_video_subtitle<R: ProgressReporterTrait>(
    reporter: &R,
    file: &Path,
    subtitle: &Path,
    srt_style: String,
) -> Result<String, String> {
    encode_video_subtitle_with_resources(reporter, file, subtitle, srt_style, Vec::new()).await
}

/// Retain the queued SRT snapshot until the media worker and renderer finish.
pub async fn encode_video_subtitle_with_resources<R: ProgressReporterTrait>(
    reporter: &R,
    file: &Path,
    subtitle: &Path,
    srt_style: String,
    resources: Vec<ffmpeg_utils::MediaResourceLease>,
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

    // Keep the source name readable and append a short ID before the extension.
    let source_name = Path::new(file_name_str(file)?);
    let stem = source_name
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("Invalid file name: {}", file.display()))?;
    let extension = source_name
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default();
    let output_filename = loop {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let name = format!(
            "{}{stem}_{}{extension}",
            constants::PREFIX_SUBTITLE,
            &id[..8]
        );
        if !file.with_file_name(&name).exists() {
            break name;
        }
    };
    let output_path = file.with_file_name(&output_filename);
    let output_guard = super::runner::TempArtifact::new(&output_path);
    encode_native_srt_with_resources(
        reporter,
        file,
        subtitle,
        &srt_style,
        &output_path,
        resources,
    )
    .await?;
    output_guard.keep();
    log::info!("Encode video subtitle task end: {}", output_path.display());
    Ok(output_filename)
}

fn native_subtitle_graph(encoder: &str) -> String {
    let output_format = hwaccel::output_format_filter(encoder);
    // FFmpeg shifts the source's common container origin, preserving relative
    // A/V delay. Never rebase only video with setpts=PTS-STARTPTS here.
    format!("[0:v]{scale},format=bgra[base];[base][1:v]overlay=format=rgb:alpha=premultiplied:eof_action=pass:repeatlast=0:shortest=0,{output_format}[video]",scale=hwaccel::H264_SCALE_PAD_FILTER)
}

#[cfg(test)]
async fn encode_native_srt<R: ProgressReporterTrait>(
    reporter: &R,
    file: &Path,
    subtitle: &Path,
    style: &str,
    output: &Path,
) -> Result<(), String> {
    encode_native_srt_with_resources(reporter, file, subtitle, style, output, Vec::new()).await
}

async fn encode_native_srt_with_resources<R: ProgressReporterTrait>(
    reporter: &R,
    file: &Path,
    subtitle: &Path,
    style: &str,
    output: &Path,
    resources: Vec<ffmpeg_utils::MediaResourceLease>,
) -> Result<(), String> {
    use super::native_overlay::{run_overlay_plan, OverlaySpec};
    use native_overlay::{AssSource, RenderInput, SourceKind};
    use tokio::io::AsyncReadExt;

    const MAX_SRT_BYTES: u64 = 64 * 1024 * 1024;
    // Freeze the SRT once; styles and events cannot change during streaming.
    let source = tokio::fs::File::open(subtitle)
        .await
        .map_err(|error| format!("Cannot read SRT {}: {error}", subtitle.display()))?;
    let mut content = String::new();
    source
        .take(MAX_SRT_BYTES + 1)
        .read_to_string(&mut content)
        .await
        .map_err(|error| format!("Cannot decode UTF-8 SRT {}: {error}", subtitle.display()))?;
    if content.len() as u64 > MAX_SRT_BYTES {
        return Err("SRT exceeds 64 MiB".into());
    }
    let ass = super::subtitle_native::srt_to_ass(&content, style)?;
    let metadata = ffmpeg_utils::extract_video_metadata(file).await?;
    let duration = super::danmu::probe_overlay_duration(file, metadata.duration).await?;
    let fps = if metadata.fps.is_finite() && metadata.fps > 0.0 {
        metadata.fps
    } else {
        30.0
    };
    let spec = OverlaySpec {
        input: RenderInput {
            documents: vec![AssSource {
                text: ass,
                kind: SourceKind::Subtitle,
            }],
            images: vec![],
        },
        width: 1920,
        height: 1080,
        fps,
        start: 0.0,
        duration,
    };
    let encoder = hwaccel::get_x264_encoder().await;
    let mut video = hwaccel::native_video_options(encoder, None, None, None)?;
    video.filter = None;
    let mut plan = super::danmu::overlay_media_plan(
        file.to_path_buf(),
        output.to_path_buf(),
        native_subtitle_graph(encoder),
        "video",
        video,
    );
    plan.resources.extend(resources);
    run_overlay_plan(
        plan,
        spec,
        1,
        ProgressMode::stage("压制字幕", ProgressTotal::FirstInput),
        "Encode native SRT subtitle",
        Some(reporter),
    )
    .await?;
    Ok(())
}

/// Decode a 16-bit PCM WAV straight to f32 samples, scaled like
/// `whisper_cpp_rs::convert_integer_to_float_audio`, so a long recording is
/// never held as an i16 copy and an f32 copy at the same time.
fn read_wav_f32(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let audio = hound::WavReader::open(path).map_err(|error| error.to_string())?;
    let sample_rate = audio.spec().sample_rate;
    // `len` is the exact sample count `into_samples` yields.
    let mut samples = Vec::with_capacity(audio.len() as usize);
    for sample in audio.into_samples::<i16>() {
        let sample = sample.map_err(|error| format!("Failed to decode WAV samples: {error}"))?;
        samples.push(f32::from(sample) / 32768.0);
    }
    Ok((samples, sample_rate))
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
    powerlive: Option<crate::subtitle_generator::powerlive::PowerLiveOptions<'_>>,
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
            let (f32_samples, sample_rate) = read_wav_f32(&full_wav)?;
            // The samples are in memory and chunks are cut from `file`, so the
            // WAV is no longer needed. The guard also removes it if decoding fails.
            drop(full_wav_guard);

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
            // Chunks are re-extracted from `file` below, so free the decoded
            // recording before the long transcription loop.
            drop(f32_samples);
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
            // The chunk directory is removed by its guard, including on the
            // early returns above.
            drop(temp_dir);

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
            let chunk_dir = super::audio::extract_audio_chunks_guarded(file).await?;
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
            let options = powerlive.ok_or("请先在设置中登录 PowerLive 账号")?;
            let generator = crate::subtitle_generator::powerlive::new(options).await?;
            let extension = file
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or_default();
            let _extracted;
            let audio_file = if matches!(
                extension.to_ascii_lowercase().as_str(),
                "opus" | "wav" | "mp3" | "m4a" | "flac"
            ) {
                file.to_path_buf()
            } else {
                let path = super::audio::extract_full_audio(file).await?;
                _extracted = TempArtifact::new(path.clone());
                path
            };
            generator
                .generate_subtitle(reporter, &audio_file, language_hint)
                .await
        }
    }
}

#[cfg(test)]
mod subtitle_tests {
    use super::*;

    #[test]
    fn native_graph_preserves_source_timeline_and_ends_with_source_video() {
        let graph = native_subtitle_graph("libx264");
        assert!(graph.starts_with(&format!(
            "[0:v]{},format=bgra[base]",
            hwaccel::H264_SCALE_PAD_FILTER
        )));
        assert!(graph.contains("alpha=premultiplied:eof_action=pass:repeatlast=0:shortest=0"));
        assert!(!graph.contains("setpts"));
        assert!(graph.ends_with("format=yuv420p[video]"));
        assert!(native_subtitle_graph("h264_vaapi").ends_with("format=nv12,hwupload[video]"));
        assert!(native_subtitle_graph("h264_videotoolbox").ends_with("format=nv12[video]"));
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg with libx264 and native system fonts"]
    async fn native_srt_burns_color_emoji_and_honors_cue_end() {
        #[derive(Clone)]
        struct Reporter;
        #[async_trait::async_trait]
        impl ProgressReporterTrait for Reporter {
            async fn update(&self, _: &str) {}
            async fn finish(&self, _: bool, _: &str) {}
        }
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("source.mp4");
        let created = ffmpeg_command()
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=black:s=320x180:r=30:d=2",
                "-c:v",
                "libx264",
                "-y",
            ])
            .arg(&input)
            .output()
            .await
            .unwrap();
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );
        let subtitle = directory.path().join("字幕.srt");
        std::fs::write(
            &subtitle,
            "1\n00:00:00,000 --> 00:00:01,000\n中文 <b>Hello</b> 😀👍🏽👩🏽‍💻🇨🇳\n",
        )
        .unwrap();
        let name = encode_video_subtitle(
            &Reporter,
            &input,
            &subtitle,
            "FontName=Arial,FontSize=24,Outline=1,Alignment=2".into(),
        )
        .await
        .unwrap();
        let output = input.with_file_name(name);
        let metadata = ffmpeg_utils::extract_video_metadata(&output).await.unwrap();
        assert_eq!((metadata.width, metadata.height), (1920, 1080));
        assert!((metadata.duration - 2.0).abs() < 0.1);
        for (time, visible) in [("0.5", true), ("1.5", false)] {
            let frame = ffmpeg_command()
                .args(["-v", "error", "-ss", time, "-i"])
                .arg(&output)
                .args([
                    "-frames:v",
                    "1",
                    "-pix_fmt",
                    "rgb24",
                    "-f",
                    "rawvideo",
                    "pipe:1",
                ])
                .output()
                .await
                .unwrap();
            assert!(frame.status.success());
            let colored = frame
                .stdout
                .chunks_exact(3)
                .filter(|p| {
                    let high = *p.iter().max().unwrap();
                    let low = *p.iter().min().unwrap();
                    high > 80 && high - low > 50
                })
                .count();
            if visible {
                assert!(colored > 100, "missing color emoji: {colored}");
            } else {
                assert_eq!(colored, 0, "subtitle persists past end");
            }
        }
    }

    fn write_pcm_wav(path: &Path, samples: &[i16]) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for &sample in samples {
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn wav_decodes_to_the_same_f32_samples_as_whisper_conversion() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.wav");
        let pcm: Vec<i16> = vec![0, 1, -1, 16384, -16384, 32767, -32768, 1234, -4321];
        write_pcm_wav(&path, &pcm);

        let mut expected = vec![0.0f32; pcm.len()];
        whisper_cpp_rs::convert_integer_to_float_audio(&pcm, &mut expected).unwrap();

        let (samples, sample_rate) = read_wav_f32(&path).unwrap();
        assert_eq!(sample_rate, 16_000);
        assert_eq!(samples, expected);
    }

    #[test]
    fn truncated_wav_reports_a_decode_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.wav");
        write_pcm_wav(&path, &(0..100).collect::<Vec<i16>>());
        // Cut the data chunk short so decoding hits EOF mid-sample.
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..bytes.len() - 51]).unwrap();

        let error = read_wav_f32(&path).unwrap_err();
        assert!(error.starts_with("Failed to decode WAV samples"), "{error}");
    }
}

#[cfg(test)]
mod native_srt_tests {
    use super::*;

    #[derive(Clone)]
    struct Reporter;
    #[async_trait::async_trait]
    impl ProgressReporterTrait for Reporter {
        async fn update(&self, _: &str) {}
        async fn finish(&self, _: bool, _: &str) {}
    }

    fn fixture() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/video/subtitle-black.mp4")
    }

    async fn red_pixels(video: &Path, time: f64, directory: &Path) -> usize {
        let image = directory.join("srt-frame.jpg");
        std::fs::write(
            &image,
            ffmpeg_utils::extract_jpeg(video, time, Some(2))
                .await
                .unwrap(),
        )
        .unwrap();
        let mut renderer = super::super::native_overlay::system_renderer(
            512,
            288,
            native_overlay::RenderInput {
                documents: Vec::new(),
                images: vec![native_overlay::ImageEvent {
                    path: image,
                    start: 0.0,
                    end: 1.0,
                    from: (0.0, 0.0),
                    to: (0.0, 0.0),
                    width: 1920.0,
                    height: 1080.0,
                    opacity: 1.0,
                }],
            },
        )
        .unwrap();
        renderer
            .frame(0.0)
            .unwrap()
            .chunks_exact(4)
            .filter(|pixel| {
                pixel[0] > 80
                    && pixel[0].saturating_sub(pixel[1]) > 50
                    && pixel[0].saturating_sub(pixel[2]) > 50
            })
            .count()
    }

    #[derive(Default)]
    struct Pcm(Vec<i16>);
    impl ffmpeg_utils::AudioSink for Pcm {
        fn write(&mut self, samples: &[i16]) -> Result<(), String> {
            self.0.extend_from_slice(samples);
            Ok(())
        }
    }

    #[tokio::test]
    async fn native_srt_renders_cue_boundaries_and_copies_audio_without_cli() {
        let directory = tempfile::tempdir().unwrap();
        let subtitle = directory.path().join("字幕 [native]'s.srt");
        std::fs::write(&subtitle, "1\n00:00:00,000 --> 00:00:00,500\nNATIVE SRT\n").unwrap();
        let output = directory.path().join("burned.mp4");
        encode_native_srt(
            &Reporter,
            &fixture(),
            &subtitle,
            "FontName=Arial,FontSize=32,PrimaryColour=&H000000FF,Outline=0",
            &output,
        )
        .await
        .unwrap();
        let metadata = ffmpeg_utils::extract_video_metadata(&output).await.unwrap();
        assert_eq!((metadata.width, metadata.height), (1920, 1080));
        assert!((metadata.duration - 1.1).abs() < 1e-6);
        assert!(red_pixels(&output, 0.2, directory.path()).await > 100);
        assert_eq!(red_pixels(&output, 0.9, directory.path()).await, 0);
        let options = ffmpeg_utils::AudioDecodeOptions {
            sample_rate: 16_000,
        };
        let expected = ffmpeg_utils::decode_audio(&fixture(), options, Pcm::default())
            .await
            .unwrap()
            .sink
            .0;
        let actual = ffmpeg_utils::decode_audio(&output, options, Pcm::default())
            .await
            .unwrap()
            .sink
            .0;
        assert_eq!(actual, expected, "native SRT must preserve source audio");
    }

    #[tokio::test]
    async fn native_srt_rejects_invalid_input_and_keeps_previous_output_without_cli() {
        let directory = tempfile::tempdir().unwrap();
        let subtitle = directory.path().join("invalid.srt");
        let output = directory.path().join("kept.mp4");
        std::fs::write(&subtitle, "invalid subtitle").unwrap();
        std::fs::write(&output, b"previous export").unwrap();
        assert!(
            encode_native_srt(&Reporter, &fixture(), &subtitle, "", &output)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&output).unwrap(), b"previous export");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
    }
}
