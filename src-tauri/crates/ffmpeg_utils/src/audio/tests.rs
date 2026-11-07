use super::*;

use std::{
    f64::consts::TAU,
    fs,
    path::Path,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

#[derive(Default)]
struct Collector {
    samples: Vec<i16>,
    finished: bool,
}

impl AudioSink for Collector {
    fn write(&mut self, samples: &[i16]) -> Result<(), String> {
        self.samples.extend_from_slice(samples);
        Ok(())
    }

    fn finish(&mut self) -> Result<(), String> {
        self.finished = true;
        Ok(())
    }
}

/// The fixture is independent of the decoder under test and needs no CLI.
fn write_pcm_wav(path: &Path, rate: u32, channels: u16, samples: &[i16]) {
    let bytes = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + bytes as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    wav.extend_from_slice(&(channels * 2).to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&bytes.to_le_bytes());
    for sample in samples {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(path, wav).unwrap();
}

fn chirp(rate: u32, frames: usize, channels: usize) -> Vec<i16> {
    (0..frames)
        .flat_map(|sample| {
            let time = sample as f64 / f64::from(rate);
            let value = (10_000.0 * (TAU * (300.0 * time + 800.0 * time * time)).sin()) as i16;
            std::iter::repeat_n(value, channels)
        })
        .collect()
}

async fn native(path: &Path, rate: u32) -> AudioDecodeResult<Collector> {
    decode_audio(
        path,
        AudioDecodeOptions { sample_rate: rate },
        Collector::default(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn pcm_decode_keeps_every_sample_including_a_partial_last_frame() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("原生 音频.wav");
    let samples: Vec<_> = (0..19_007)
        .map(|index| ((index * 997) as u16) as i16)
        .collect();
    write_pcm_wav(&source, 16_000, 1, &samples);
    let decoded = native(&source, 16_000).await;
    assert!(decoded.has_audio);
    assert!(decoded.sink.finished);
    assert_eq!(decoded.samples, samples.len() as u64);
    assert_eq!(decoded.sink.samples, samples);
}

#[tokio::test]
async fn resampling_drains_the_tail_for_up_and_noninteger_downsampling() {
    let directory = tempfile::tempdir().unwrap();
    for (input_rate, frames, output_rate, expected_frames) in [
        (8_000, 1_001, 16_000, 2_002),
        (44_100, 44_107, 16_000, 16_003),
        (48_000, 24_001, 8_000, 4_000),
    ] {
        let source = directory.path().join(format!("{input_rate}.wav"));
        let samples = vec![8_000; frames];
        write_pcm_wav(&source, input_rate, 1, &samples);
        let decoded = native(&source, output_rate).await;
        assert_eq!(decoded.samples, expected_frames);
        assert_eq!(decoded.sink.samples.len() as u64, expected_frames);
        // Constant input makes a missing/zero-padded resampler tail visible.
        assert!(decoded
            .sink
            .samples
            .iter()
            .all(|sample| sample.abs_diff(8_000) <= 1));
        assert!(decoded.sink.finished);
    }
}

#[tokio::test]
async fn stereo_downmix_uses_both_channels_without_clipping() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("stereo.wav");
    let samples: Vec<_> = (0..4_097).flat_map(|_| [12_000, -4_000]).collect();
    write_pcm_wav(&source, 16_000, 2, &samples);
    let decoded = native(&source, 16_000).await;
    assert_eq!(decoded.samples, 4_097);
    assert!(decoded
        .sink
        .samples
        .iter()
        .all(|sample| sample.abs_diff(4_000) <= 1));
}

#[tokio::test]
async fn missing_audio_is_distinct_from_invalid_media() {
    let video = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video/test.mp4");
    let decoded = native(&video, 8_000).await;
    assert!(!decoded.has_audio);
    assert_eq!(decoded.samples, 0);
    assert!(decoded.sink.samples.is_empty());
    assert!(decoded.sink.finished);

    let directory = tempfile::tempdir().unwrap();
    let invalid = directory.path().join("invalid.wav");
    fs::write(&invalid, b"not a media file").unwrap();
    assert!(decode_audio(
        &invalid,
        AudioDecodeOptions {
            sample_rate: 16_000
        },
        Collector::default(),
    )
    .await
    .is_err());
}

#[tokio::test]
async fn consumer_failures_are_reported_without_finalizing() {
    struct FailingSink(Arc<AtomicBool>);
    impl AudioSink for FailingSink {
        fn write(&mut self, _: &[i16]) -> Result<(), String> {
            Err("test consumer refused samples".to_string())
        }
        fn finish(&mut self) -> Result<(), String> {
            self.0.store(true, Ordering::Release);
            Ok(())
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("tone.wav");
    write_pcm_wav(&source, 16_000, 1, &vec![1_000; 16_000]);
    let finished = Arc::new(AtomicBool::new(false));
    let result = decode_audio(
        &source,
        AudioDecodeOptions { sample_rate: 8_000 },
        FailingSink(finished.clone()),
    )
    .await;
    assert!(result
        .err()
        .unwrap()
        .contains("test consumer refused samples"));
    assert!(!finished.load(Ordering::Acquire));
}

#[tokio::test]
async fn decoder_failure_after_partial_output_does_not_finalize_the_consumer() {
    struct TrackingSink {
        received: Arc<AtomicUsize>,
        finished: Arc<AtomicBool>,
    }
    impl AudioSink for TrackingSink {
        fn write(&mut self, samples: &[i16]) -> Result<(), String> {
            self.received.fetch_add(samples.len(), Ordering::Relaxed);
            Ok(())
        }
        fn finish(&mut self) -> Result<(), String> {
            self.finished.store(true, Ordering::Release);
            Ok(())
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("truncated-sample.wav");
    write_pcm_wav(&source, 16_000, 1, &vec![1_000; 40_000]);
    let mut contents = fs::read(&source).unwrap();
    // Advertise an odd payload length: the final PCM packet has one byte but
    // needs two for its 16-bit sample. Earlier packets remain valid audio.
    contents.push(1);
    let riff_size = (contents.len() - 8) as u32;
    let data_size = (contents.len() - 44) as u32;
    contents[4..8].copy_from_slice(&riff_size.to_le_bytes());
    contents[40..44].copy_from_slice(&data_size.to_le_bytes());
    fs::write(&source, contents).unwrap();
    let received = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicBool::new(false));
    let result = decode_audio(
        &source,
        AudioDecodeOptions {
            sample_rate: 16_000,
        },
        TrackingSink {
            received: received.clone(),
            finished: finished.clone(),
        },
    )
    .await;
    assert!(result.is_err());
    assert!(received.load(Ordering::Relaxed) > 0);
    assert!(!finished.load(Ordering::Acquire));
}

#[tokio::test]
async fn cancelling_decode_drops_the_worker_owned_consumer() {
    struct SlowSink {
        started: Option<tokio::sync::oneshot::Sender<()>>,
        stopped: Option<tokio::sync::oneshot::Sender<()>>,
        finished: Arc<AtomicBool>,
    }
    impl AudioSink for SlowSink {
        fn write(&mut self, _: &[i16]) -> Result<(), String> {
            if let Some(started) = self.started.take() {
                let _ = started.send(());
                std::thread::sleep(Duration::from_millis(200));
            }
            Ok(())
        }
        fn finish(&mut self) -> Result<(), String> {
            self.finished.store(true, Ordering::Release);
            Ok(())
        }
    }
    impl Drop for SlowSink {
        fn drop(&mut self) {
            if let Some(stopped) = self.stopped.take() {
                let _ = stopped.send(());
            }
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("cancel.wav");
    write_pcm_wav(&source, 48_000, 1, &vec![1_000; 144_000]);
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (stopped_tx, stopped_rx) = tokio::sync::oneshot::channel();
    let finished = Arc::new(AtomicBool::new(false));
    let sink = SlowSink {
        started: Some(started_tx),
        stopped: Some(stopped_tx),
        finished: finished.clone(),
    };
    let task = tokio::spawn(async move {
        decode_audio(
            &source,
            AudioDecodeOptions {
                sample_rate: 16_000,
            },
            sink,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), started_rx)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    tokio::time::timeout(Duration::from_secs(5), stopped_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(!finished.load(Ordering::Acquire));
}

fn cli_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn ffmpeg(args: &[&str], output: &Path) {
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-y"])
        .args(args)
        .arg(output)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn cli_pcm(path: &Path, map: Option<&str>) -> Vec<i16> {
    let mut command = Command::new("ffmpeg");
    command
        .args(["-v", "error", "-nostdin"])
        .arg("-i")
        .arg(path);
    if let Some(map) = map {
        command.args(["-map", map]);
    }
    let output = command
        .args([
            "-ar",
            "16000",
            "-ac",
            "1",
            "-vn",
            "-c:a",
            "pcm_s16le",
            "-f",
            "s16le",
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.len() % 2, 0);
    output
        .stdout
        .chunks_exact(2)
        .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
        .collect()
}

fn assert_pcm_matches(actual: &[i16], expected: &[i16], context: &str) {
    assert!(
        actual.len().abs_diff(expected.len()) <= 1,
        "{context}: sample counts {} != {}",
        actual.len(),
        expected.len()
    );
    let maximum_error = actual
        .iter()
        .zip(expected)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    assert!(
        maximum_error <= 2,
        "{context}: maximum PCM error {maximum_error}; counts {} / {}",
        actual.len(),
        expected.len()
    );
}

#[tokio::test]
async fn aac_mp3_and_flv_full_audio_match_the_previous_cli_when_available() {
    if !cli_available() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.wav");
    write_pcm_wav(&source, 48_000, 1, &chirp(48_000, 144_000, 1));
    for (name, codec) in [
        ("audio.m4a", "aac"),
        ("audio.mp3", "libmp3lame"),
        ("audio.flv", "aac"),
    ] {
        let encoded = directory.path().join(name);
        ffmpeg(
            &[
                "-i",
                source.to_str().unwrap(),
                "-c:a",
                codec,
                "-b:a",
                "128k",
            ],
            &encoded,
        );
        let full = native(&encoded, 16_000).await;
        assert_pcm_matches(&full.sink.samples, &cli_pcm(&encoded, None), name);
    }
}

#[tokio::test]
async fn automatic_audio_selection_matches_cli_instead_of_find_best_stream_when_available() {
    if !cli_available() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let mono = directory.path().join("mono.wav");
    let stereo = directory.path().join("stereo.wav");
    write_pcm_wav(&mono, 48_000, 1, &vec![8_000; 48_000]);
    write_pcm_wav(&stereo, 48_000, 2, &vec![-8_000; 96_000]);
    let multi = directory.path().join("multiple.mkv");
    ffmpeg(
        &[
            "-i",
            mono.to_str().unwrap(),
            "-i",
            stereo.to_str().unwrap(),
            "-map",
            "0:a",
            "-map",
            "1:a",
            "-c:a",
            "aac",
            "-b:a:0",
            "192k",
            "-b:a:1",
            "32k",
            "-disposition:a:0",
            "0",
            "-disposition:a:1",
            "0",
        ],
        &multi,
    );
    let expected = cli_pcm(&multi, None);
    assert_eq!(expected, cli_pcm(&multi, Some("0:a:1")));
    assert_ne!(expected, cli_pcm(&multi, Some("0:a:0")));
    assert_pcm_matches(
        &native(&multi, 16_000).await.sink.samples,
        &expected,
        "automatic multi-track selection",
    );
}

#[tokio::test]
async fn full_audio_keeps_samples_with_shifted_container_and_delayed_audio_when_cli_is_available() {
    if !cli_available() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.wav");
    write_pcm_wav(&source, 48_000, 1, &chirp(48_000, 144_000, 1));
    let shifted = directory.path().join("shifted.m4a");
    ffmpeg(
        &[
            "-i",
            source.to_str().unwrap(),
            "-af",
            "asetpts=PTS+2/TB",
            "-c:a",
            "aac",
        ],
        &shifted,
    );
    let delayed = directory.path().join("delayed.mp4");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=black:s=32x32:r=10:d=4",
            "-itsoffset",
            "1",
            "-i",
            source.to_str().unwrap(),
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
        ],
        &delayed,
    );
    for path in [&shifted, &delayed] {
        assert_pcm_matches(
            &native(path, 16_000).await.sink.samples,
            &cli_pcm(path, None),
            &format!("{} full audio", path.display()),
        );
    }
}
