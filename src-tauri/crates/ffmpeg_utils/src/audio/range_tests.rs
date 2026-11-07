use super::*;

use std::{
    f64::consts::TAU,
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
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

fn pcm_wav(path: &Path, rate: u32, samples: &[i16]) {
    let bytes = (samples.len() * 2) as u32;
    let mut data = Vec::with_capacity(44 + bytes as usize);
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&(36 + bytes).to_le_bytes());
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16u32.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&rate.to_le_bytes());
    data.extend_from_slice(&(rate * 2).to_le_bytes());
    data.extend_from_slice(&2u16.to_le_bytes());
    data.extend_from_slice(&16u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&bytes.to_le_bytes());
    for sample in samples {
        data.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(path, data).unwrap();
}

fn chirp(rate: u32, seconds: usize) -> Vec<i16> {
    (0..rate as usize * seconds)
        .map(|index| {
            let time = index as f64 / f64::from(rate);
            (10_000.0 * (TAU * (300.0 * time + 800.0 * time * time)).sin()) as i16
        })
        .collect()
}

async fn native(path: &Path, start: f64, duration: f64) -> AudioDecodeResult<Collector> {
    let result = decode_audio_range(
        path,
        AudioDecodeOptions {
            sample_rate: 16_000,
        },
        AudioRange { start, duration },
        Collector::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.samples, result.sink.samples.len() as u64);
    assert!(result.sink.finished);
    result
}

#[tokio::test]
async fn pcm_ranges_preserve_exact_samples_zero_duration_and_eof_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("精确 范围.wav");
    let samples: Vec<_> = (0..32_003)
        .map(|index| (index * 977) as u16 as i16)
        .collect();
    pcm_wav(&source, 16_000, &samples);
    let result = native(&source, 0.125, 0.777).await;
    assert!(result.has_audio);
    assert_identical(
        &result.sink.samples,
        &samples[2_000..14_432],
        "PCM sample boundaries",
    );
    assert!(native(&source, 0.0, 0.0).await.sink.samples.is_empty());
    assert!(native(&source, 5.0, 0.5).await.sink.samples.is_empty());
}

#[tokio::test]
async fn recorded_fmp4_aac_ranges_keep_the_container_clock_without_a_cli() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video");
    let directory = tempfile::tempdir().unwrap();
    let joined = directory.path().join("recorded.mp4");
    let mut contents = fs::read(fixtures.join("init.m4s")).unwrap();
    contents.extend(fs::read(fixtures.join("segment.m4s")).unwrap());
    fs::write(&joined, contents).unwrap();
    // This real live fragment starts at t=206.005s. -ss is relative to that
    // container origin, while its format duration still carries absolute PTS.
    let tiny = native(&joined, 0.111, 0.001).await;
    assert!(tiny.has_audio);
    assert_eq!(tiny.samples, 16);
    assert!(tiny.sink.samples.iter().any(|sample| *sample != 0));
    let range = native(&joined, 0.111, 0.777).await;
    assert_eq!(range.samples, 12_443);
    assert!(range.sink.samples.iter().any(|sample| *sample != 0));
    assert!(native(&joined, 2.0, 0.5).await.sink.samples.is_empty());
}

#[tokio::test]
async fn missing_audio_and_invalid_ranges_are_distinct_without_a_cli() {
    let video = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video/test.mp4");
    let result = native(&video, 0.0, 0.777).await;
    assert!(!result.has_audio);
    assert!(result.sink.samples.is_empty());
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("input.wav");
    pcm_wav(&source, 16_000, &[1; 100]);
    for (start, duration) in [
        (-1.0, 0.5),
        (f64::NAN, 0.5),
        (f64::INFINITY, 0.5),
        (0.0, -1.0),
        (0.0, f64::NAN),
        (0.0, f64::INFINITY),
        (f64::MAX, f64::MAX),
    ] {
        assert!(
            decode_audio_range(
                &source,
                AudioDecodeOptions {
                    sample_rate: 16_000,
                },
                AudioRange { start, duration },
                Collector::default(),
            )
            .await
            .is_err(),
            "accepted start={start}, duration={duration}"
        );
    }
}

#[tokio::test]
async fn cancelling_a_range_drops_unfinished_worker_output_without_a_cli() {
    struct GuardedSink {
        temporary: Option<tempfile::NamedTempFile>,
        started: Option<tokio::sync::oneshot::Sender<()>>,
        dropped: Option<tokio::sync::oneshot::Sender<()>>,
        finished: Arc<AtomicBool>,
    }
    impl AudioSink for GuardedSink {
        fn write(&mut self, samples: &[i16]) -> Result<(), String> {
            let Some(sample) = samples.first() else {
                return Ok(());
            };
            let bytes = sample.to_le_bytes();
            self.temporary
                .as_mut()
                .unwrap()
                .write_all(&bytes)
                .map_err(|error| error.to_string())?;
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
    impl Drop for GuardedSink {
        fn drop(&mut self) {
            self.temporary.take();
            if let Some(dropped) = self.dropped.take() {
                let _ = dropped.send(());
            }
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("cancel.wav");
    pcm_wav(&source, 48_000, &vec![8_000; 144_000]);
    let temporary = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
    let temporary_path = temporary.path().to_path_buf();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    let finished = Arc::new(AtomicBool::new(false));
    let sink = GuardedSink {
        temporary: Some(temporary),
        started: Some(started_tx),
        dropped: Some(dropped_tx),
        finished: finished.clone(),
    };
    let task = tokio::spawn(async move {
        decode_audio_range(
            &source,
            AudioDecodeOptions {
                sample_rate: 16_000,
            },
            AudioRange {
                start: 0.0,
                duration: 2.5,
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
    tokio::time::timeout(Duration::from_secs(5), dropped_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(!temporary_path.exists());
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

fn encode(args: &[&str], output: &Path) {
    let result = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-y"])
        .args(args)
        .arg(output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn wav_samples(path: &Path) -> Vec<i16> {
    let data = fs::read(path).unwrap();
    assert_eq!(&data[..4], b"RIFF");
    assert_eq!(&data[8..12], b"WAVE");
    let mut cursor = 12;
    let mut samples = None;
    let mut format = false;
    while cursor + 8 <= data.len() {
        let size = u32::from_le_bytes(data[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        let contents = &data[cursor + 8..cursor + 8 + size];
        match &data[cursor..cursor + 4] {
            b"fmt " => {
                assert_eq!(u16::from_le_bytes(contents[0..2].try_into().unwrap()), 1);
                assert_eq!(u16::from_le_bytes(contents[2..4].try_into().unwrap()), 1);
                assert_eq!(
                    u32::from_le_bytes(contents[4..8].try_into().unwrap()),
                    16_000
                );
                assert_eq!(u16::from_le_bytes(contents[14..16].try_into().unwrap()), 16);
                format = true;
            }
            b"data" => {
                assert_eq!(contents.len() % 2, 0);
                samples = Some(
                    contents
                        .chunks_exact(2)
                        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                        .collect(),
                );
            }
            _ => (),
        }
        cursor += 8 + size + size % 2;
    }
    assert!(format);
    samples.unwrap()
}

fn cli_range(path: &Path, start: f64, duration: f64, output: &Path) -> Vec<i16> {
    encode(
        &[
            "-ss",
            &start.to_string(),
            "-i",
            path.to_str().unwrap(),
            "-t",
            &duration.to_string(),
            "-ar",
            "16000",
            "-ac",
            "1",
            "-c:a",
            "pcm_s16le",
            "-vn",
        ],
        output,
    );
    wav_samples(output)
}

fn assert_identical(actual: &[i16], expected: &[i16], context: &str) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "{context}: sample count differs"
    );
    let first = actual.iter().zip(expected).position(|(a, b)| a != b);
    if let Some(index) = first {
        let maximum = actual
            .iter()
            .zip(expected)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        panic!(
            "{context}: first PCM difference at {index}: {} != {}; maximum difference {maximum}",
            actual[index], expected[index]
        );
    }
}

async fn compare_ranges(path: &Path, cases: &[(f64, f64)], output: &Path) {
    let mut cases = cases.to_vec();
    if let Some(end) = crate::probe_media(path).await.unwrap().container_duration {
        // Container duration may end before AAC's final decoded packet. A
        // range starting exactly there still needs to try the demuxer/decoder.
        cases.extend([(end, 0.5), (end + 0.01, 0.5), (end + 1.0, 0.5)]);
    }
    for (start, duration) in cases {
        let expected = cli_range(path, start, duration, output);
        let decoded = native(path, start, duration).await;
        assert!(decoded.has_audio);
        assert_identical(
            &decoded.sink.samples,
            &expected,
            &format!("{}: -ss {start} -t {duration}", path.display()),
        );
    }
}

const CASES: &[(f64, f64)] = &[
    (0.0, 0.777),
    (0.111, 1.123),
    // One 16 kHz sample has seven decimal places. Preserve the CLI parser's
    // microsecond handling rather than rounding f64 timestamps ourselves.
    (0.000_062_5, 0.777),
    (0.0, 0.001),
    (0.111, 0.001),
    (0.021_333, 0.021_333),
    (0.333, 0.017_777),
    (0.499, 0.050_001),
    (0.999, 1.111),
    (2.777, 0.777),
    (3.0, 0.125),
    (6.0, 0.125),
    (0.0, 0.0),
];

#[tokio::test]
async fn aac_m4a_mp4_flv_and_adts_ranges_match_every_cli_sample_when_available() {
    if !cli_available() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("oracle.wav");
    for rate in [48_000, 44_100] {
        let source = directory.path().join(format!("source-{rate}.wav"));
        pcm_wav(&source, rate, &chirp(rate, 3));
        for extension in ["m4a", "mp4", "flv", "aac"] {
            let encoded = directory.path().join(format!("audio-{rate}.{extension}"));
            let source_name = source.to_str().unwrap();
            let mut args = vec!["-i", source_name];
            if extension == "mp4" {
                args.extend([
                    "-f",
                    "lavfi",
                    "-i",
                    "color=black:s=32x32:r=25:d=3",
                    "-map",
                    "1:v",
                    "-map",
                    "0:a",
                    "-c:v",
                    "mpeg4",
                    "-bf",
                    "2",
                ]);
            }
            args.extend(["-c:a", "aac", "-b:a", "128k"]);
            encode(&args, &encoded);
            if rate == 48_000 && extension == "m4a" {
                let known = cli_range(&encoded, 0.0, 0.777, &output);
                // This case produced 12272 samples with FFmpeg 9, but 12432
                // with 6.1. Keep the regression strict against the matching
                // SDK's CLI instead of imposing one release's scheduling.
                let decoded = native(&encoded, 0.0, 0.777).await;
                assert_identical(
                    &decoded.sink.samples,
                    &known,
                    "AAC seek-zero scheduling regression",
                );
            }
            compare_ranges(&encoded, CASES, &output).await;
        }
    }
}

#[tokio::test]
async fn shifted_and_delayed_audio_ranges_match_cli_without_padding_when_available() {
    if !cli_available() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.wav");
    pcm_wav(&source, 48_000, &chirp(48_000, 3));
    let shifted = directory.path().join("shifted.m4a");
    encode(
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
    encode(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=black:s=32x32:r=25:d=4",
            "-itsoffset",
            "1",
            "-i",
            source.to_str().unwrap(),
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-c:v",
            "mpeg4",
            "-bf",
            "2",
            "-c:a",
            "aac",
        ],
        &delayed,
    );
    let output = directory.path().join("oracle.wav");
    for path in [&shifted, &delayed] {
        compare_ranges(path, CASES, &output).await;
    }
    assert!(native(&delayed, 0.0, 0.777).await.sink.samples.is_empty());
}

#[tokio::test]
async fn pcm_and_mp3_ranges_match_every_cli_sample_when_available() {
    if !cli_available() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("oracle.wav");
    for rate in [48_000, 44_100] {
        let source = directory.path().join(format!("source-{rate}.wav"));
        pcm_wav(&source, rate, &chirp(rate, 3));
        let mp3 = directory.path().join(format!("audio-{rate}.mp3"));
        encode(
            &[
                "-i",
                source.to_str().unwrap(),
                "-c:a",
                "libmp3lame",
                "-b:a",
                "128k",
            ],
            &mp3,
        );
        for path in [&source, &mp3] {
            compare_ranges(
                path,
                &[(0.0, 0.777), (0.111, 1.123), (2.777, 0.777), (5.0, 0.125)],
                &output,
            )
            .await;
        }
    }
}

#[tokio::test]
async fn aac_sample_rate_changes_rebuild_the_range_graph_like_cli_when_available() {
    if !cli_available() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let mut joined = Vec::new();
    for rate in [48_000, 44_100] {
        let source = directory.path().join(format!("source-{rate}.wav"));
        pcm_wav(&source, rate, &chirp(rate, 1));
        let part = directory.path().join(format!("part-{rate}.aac"));
        encode(
            &[
                "-i",
                source.to_str().unwrap(),
                "-t",
                "0.5",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
            ],
            &part,
        );
        joined.extend(fs::read(part).unwrap());
    }
    let source = directory.path().join("changing-rate.aac");
    fs::write(&source, joined).unwrap();
    compare_ranges(
        &source,
        &[(0.0, 0.9), (0.111, 0.777), (0.4, 0.777), (0.65, 0.1)],
        &directory.path().join("oracle.wav"),
    )
    .await;
}
