use std::path::{Path, PathBuf};

use tempfile::{tempdir_in, TempDir};

use super::runner::{FfmpegJob, ProgressMode};
use ffmpeg_utils::{probe_media, AudioStreamMetadata};

/// Extract an audio sample for waveform display.
pub async fn extract_audio_sample(file: &Path) -> Result<PathBuf, String> {
    log::info!("Extract audio sample task start: {}", file.display());
    let output_path = file.with_extension("opus");
    let job = FfmpegJob::new()
        .args([
            "-i".to_string(),
            file.to_string_lossy().into_owned(),
            "-c:a".to_string(),
            "libopus".to_string(),
            "-ar".to_string(),
            "16000".to_string(),
            "-ac".to_string(),
            "1".to_string(),
            "-vn".to_string(),
            "-b:a".to_string(),
            "64k".to_string(),
            "-vbr".to_string(),
            "on".to_string(),
            "-compression_level".to_string(),
            "10".to_string(),
            "-y".to_string(),
        ])
        .output(output_path.clone())
        .progress(ProgressMode::Time)
        .context("Extract audio sample");
    job.run_without_reporter()
        .await
        .map_err(|error| error.to_string())?;

    if output_path.exists() {
        log::info!("Extract audio sample task end: {}", output_path.display());
        Ok(output_path)
    } else {
        Err("Audio sample extraction failed: output file not found".to_string())
    }
}

/// Extract 16 kHz mono audio chunks into an RAII-managed temporary directory.
///
/// The directory is deleted when the returned [`TempDir`] is dropped, so a
/// failed or cancelled transcription cannot leave chunk files behind.
pub(crate) async fn extract_audio_chunks_guarded(
    file: &Path,
    format: &str,
) -> Result<TempDir, String> {
    log::info!("Extract audio task start: {}", file.display());
    let output_path = file.with_extension(format);
    let output_dir = output_path
        .parent()
        .ok_or_else(|| format!("Output path has no parent: {}", output_path.display()))?;
    std::fs::create_dir_all(output_dir)
        .map_err(|error| format!("Failed to create output directory: {error}"))?;

    let duration = get_audio_duration(file).await?;
    log::info!("Audio duration: {duration} seconds");
    let chunk_duration = 30;
    let chunk_count = (duration as f64 / f64::from(chunk_duration)).ceil() as usize;
    log::info!("Splitting into {chunk_count} chunks of {chunk_duration} seconds each");

    let chunk_dir = tempdir_in(output_dir)
        .map_err(|error| format!("Failed to create chunk directory: {error}"))?;
    let base_name = output_path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Invalid output file name: {}", output_path.display()))?;
    let segment_pattern = chunk_dir.path().join(format!("{base_name}_%03d.{format}"));

    let mut args = vec![
        "-i".to_string(),
        file.to_string_lossy().into_owned(),
        "-ar".to_string(),
        "16000".to_string(),
        "-ac".to_string(),
        "1".to_string(),
        "-vn".to_string(),
        "-f".to_string(),
        "segment".to_string(),
        "-segment_time".to_string(),
        chunk_duration.to_string(),
        "-reset_timestamps".to_string(),
        "1".to_string(),
        "-y".to_string(),
    ];
    if format == "mp3" {
        args.extend([
            "-c:a".to_string(),
            "mp3".to_string(),
            "-b:a".to_string(),
            "64k".to_string(),
            "-compression_level".to_string(),
            "0".to_string(),
        ]);
    } else {
        args.extend(["-c:a".to_string(), "pcm_s16le".to_string()]);
    }
    args.extend(["-threads".to_string(), "0".to_string()]);

    FfmpegJob::with_args(args)
        .output(segment_pattern)
        .progress(ProgressMode::Time)
        .context("Extract audio chunks")
        .run_without_reporter()
        .await
        .map_err(|error| error.to_string())?;

    log::info!(
        "Extract audio task end: {} chunks created in {}",
        chunk_count,
        chunk_dir.path().display()
    );
    Ok(chunk_dir)
}

/// Extract the full audio track as a single 16 kHz mono WAV file.
pub async fn extract_full_audio(file: &Path) -> Result<PathBuf, String> {
    log::info!("Extract full audio: {}", file.display());
    let output_path = file.with_extension("full.wav");
    let job = FfmpegJob::new()
        .args([
            "-i".to_string(),
            file.to_string_lossy().into_owned(),
            "-ar".to_string(),
            "16000".to_string(),
            "-ac".to_string(),
            "1".to_string(),
            "-c:a".to_string(),
            "pcm_s16le".to_string(),
            "-vn".to_string(),
            "-y".to_string(),
        ])
        .output(output_path.clone())
        .progress(ProgressMode::Time)
        .context("Extract full audio");

    if let Err(error) = job.run_without_reporter().await {
        let _ = tokio::fs::remove_file(&output_path).await;
        return Err(error.to_string());
    }
    if output_path.exists() {
        log::info!("Full audio extracted: {}", output_path.display());
        Ok(output_path)
    } else {
        Err("Full audio extraction failed: output file not found".to_string())
    }
}

/// Extract a time segment as a 16 kHz mono WAV file.
pub(crate) async fn extract_audio_segment(
    file: &Path,
    start_sec: f64,
    duration_sec: f64,
    output_path: &Path,
) -> Result<(), String> {
    let job = FfmpegJob::new()
        .args([
            "-ss".to_string(),
            start_sec.to_string(),
            "-i".to_string(),
            file.to_string_lossy().into_owned(),
            "-t".to_string(),
            duration_sec.to_string(),
            "-ar".to_string(),
            "16000".to_string(),
            "-ac".to_string(),
            "1".to_string(),
            "-c:a".to_string(),
            "pcm_s16le".to_string(),
            "-vn".to_string(),
            "-y".to_string(),
        ])
        .output(output_path.to_path_buf())
        .progress(ProgressMode::Time)
        .context("Extract audio segment");

    if let Err(error) = job.run_without_reporter().await {
        let _ = tokio::fs::remove_file(output_path).await;
        return Err(error.to_string());
    }
    if output_path.exists() {
        Ok(())
    } else {
        Err("Audio segment extraction failed: output file not found".to_string())
    }
}

/// How the samples of a file's sound are placed in time, which cutting it
/// sample for sample depends on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AudioTiming {
    pub sample_rate: u32,
    /// Where its first sample is, in samples: a decode of the whole file
    /// starts there, as after the priming an MP3 encoder leaves.
    pub start: i64,
    /// AAC, whose frames all hold 1024 samples. Recordings remuxed from FLV
    /// stamp them to the millisecond, up to 16 samples off where they are;
    /// snapped back to their grid they are exact.
    pub aac: bool,
}

pub(crate) const AAC_FRAME_SAMPLES: i64 = 1024;

/// The timing of a file's first sound stream; `None` without one.
pub(crate) async fn probe_audio_timing(file: &Path) -> Result<Option<AudioTiming>, String> {
    Ok(probe_media(file)
        .await?
        .audio
        .as_ref()
        .and_then(audio_timing))
}

fn audio_timing(audio: &AudioStreamMetadata) -> Option<AudioTiming> {
    let sample_rate = audio.sample_rate;
    if sample_rate == 0 {
        return None;
    }
    let start = audio.start_time.unwrap_or(0.0);
    Some(AudioTiming {
        sample_rate,
        start: (start * f64::from(sample_rate)).round() as i64,
        aac: audio.codec == "aac",
    })
}

/// The filters cutting `duration` seconds from `start`, counted from the
/// first sample a decode of the whole file has, as 48 kHz in frames of 4096
/// samples.
///
/// The input keeps its timestamps (`-copyts`) and is decoded from a whole
/// frame (`-noaccurate_seek`). Its first frame is placed by its timestamp,
/// on its grid for AAC, the rest counted on from it, so `atrim` cuts at the
/// very sample a decode of the whole file has there, and chunks cut next to
/// each other join sample for sample. Frames of a fixed size, whatever the
/// decoder gave, keep the FLAC encoder from writing a stream of varying block
/// sizes, which Apple's decoder, and so WebKit's, cannot read.
fn preview_audio_filters(timing: AudioTiming, start: f64, duration: f64) -> String {
    let rate = timing.sample_rate;
    let origin = timing.start;
    let first = if timing.aac {
        format!("round((STARTPTS-({origin}))/{AAC_FRAME_SAMPLES})*{AAC_FRAME_SAMPLES}")
    } else {
        format!("round(STARTPTS)-({origin})")
    };
    let from = (start * f64::from(rate)).round() as i64;
    let to = from + (duration * f64::from(rate)).round() as i64;
    format!(
        "asettb=1/{rate},asetpts='{first}+N',atrim=start_pts={from}:end_pts={to},\
         asetpts=PTS-STARTPTS,aresample=48000,asetnsamples=n=4096:p=0"
    )
}

/// Cut `duration` seconds of a file's sound from `start` as 48 kHz stereo
/// 16-bit FLAC for the editor's preview, decoded from up to `pre_roll`
/// seconds before so that its first samples are as in the whole file.
pub(crate) async fn extract_preview_audio(
    file: &Path,
    timing: AudioTiming,
    start: f64,
    duration: f64,
    pre_roll: f64,
    output_path: &Path,
) -> Result<(), String> {
    // From the start, unseeked: a seek to it drops the priming frame of AAC
    // with the first one after it, which a decode of the whole file keeps.
    let seek = start - pre_roll;
    let seek = (seek > 0.0)
        .then(|| ["-ss".to_string(), format!("{seek:.6}")])
        .into_iter()
        .flatten();
    let job = FfmpegJob::new()
        .args(["-noaccurate_seek".to_string(), "-copyts".to_string()])
        .args(seek)
        .args([
            "-i".to_string(),
            file.to_string_lossy().into_owned(),
            "-map".to_string(),
            "0:a:0".to_string(),
            "-af".to_string(),
            preview_audio_filters(timing, start, duration),
            "-ac".to_string(),
            "2".to_string(),
            "-c:a".to_string(),
            "flac".to_string(),
            "-sample_fmt".to_string(),
            "s16".to_string(),
            "-f".to_string(),
            "flac".to_string(),
            "-y".to_string(),
        ])
        .output(output_path.to_path_buf())
        .context("Extract preview audio");
    job.run_without_reporter()
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Get the container duration in whole seconds, rounding up for audio chunks.
pub(crate) async fn get_audio_duration(file: &Path) -> Result<u64, String> {
    probe_media(file)
        .await?
        .container_duration
        .map(|seconds| seconds.ceil() as u64)
        .ok_or_else(|| "Failed to read container duration".to_string())
}

#[cfg(test)]
mod preview_audio_tests {
    use super::*;

    #[test]
    fn sound_is_counted_from_its_first_sample_and_aac_put_on_its_frame_grid() {
        let aac = audio_timing(&AudioStreamMetadata {
            codec: "aac".to_string(),
            sample_rate: 48000,
            start_time: Some(0.0),
        });
        assert_eq!(
            aac,
            Some(AudioTiming {
                sample_rate: 48000,
                start: 0,
                aac: true,
            })
        );
        // The priming an MP3 encoder leaves.
        let mp3 = audio_timing(&AudioStreamMetadata {
            codec: "mp3".to_string(),
            sample_rate: 44100,
            start_time: Some(0.025057),
        })
        .unwrap();
        assert_eq!((mp3.start, mp3.aac), (1105, false));
        let unknown = audio_timing(&AudioStreamMetadata {
            codec: "opus".to_string(),
            sample_rate: 48000,
            start_time: None,
        })
        .unwrap();
        assert_eq!(unknown.start, 0);
        assert_eq!(
            audio_timing(&AudioStreamMetadata {
                codec: "aac".to_string(),
                sample_rate: 0,
                start_time: None,
            }),
            None
        );

        assert_eq!(
            preview_audio_filters(aac.unwrap(), 50.0, 10.0),
            "asettb=1/48000,asetpts='round((STARTPTS-(0))/1024)*1024+N',\
             atrim=start_pts=2400000:end_pts=2880000,asetpts=PTS-STARTPTS,\
             aresample=48000,asetnsamples=n=4096:p=0"
        );
        assert!(preview_audio_filters(mp3, 10.0, 10.0)
            .starts_with("asettb=1/44100,asetpts='round(STARTPTS)-(1105)+N',atrim=start_pts=441000:end_pts=882000"));
    }

    #[tokio::test]
    async fn native_audio_probe_keeps_sample_rate_duration_and_silent_video_behavior() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        let wav = fixtures.join("audio/test.wav");
        assert_eq!(
            probe_audio_timing(&wav).await.unwrap(),
            Some(AudioTiming {
                sample_rate: 16000,
                start: 0,
                aac: false,
            })
        );
        assert_eq!(get_audio_duration(&wav).await.unwrap(), 84);
        assert_eq!(
            probe_audio_timing(&fixtures.join("video/test.mp4"))
                .await
                .unwrap(),
            None
        );
    }
}
