use std::{
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
};

use tempfile::{tempdir_in, Builder, NamedTempFile, TempDir};

pub(crate) use ffmpeg_utils::AudioTiming;
use ffmpeg_utils::{
    decode_audio, decode_audio_range, encode_flac_preview, encode_opus_sample, probe_media,
    AudioDecodeOptions, AudioRange, AudioSink, AudioStreamMetadata,
};

const TRANSCRIPTION_SAMPLE_RATE: u32 = 16_000;
const CHUNK_SAMPLES: usize = TRANSCRIPTION_SAMPLE_RATE as usize * 30;

/// Encode the material's 16 kHz mono Opus sidecar.
pub async fn extract_audio_sample(file: &Path) -> Result<PathBuf, String> {
    log::info!("Extract audio sample task start: {}", file.display());
    let output_path = file.with_extension("opus");
    encode_opus_sample(file, &output_path).await?;
    log::info!("Extract audio sample task end: {}", output_path.display());
    Ok(output_path)
}

/// Extract 16 kHz mono WAV chunks into an RAII-managed temporary directory.
///
/// The directory is deleted when the returned [`TempDir`] is dropped, so a
/// failed or cancelled transcription cannot leave chunk files behind.
pub(crate) async fn extract_audio_chunks_guarded(file: &Path) -> Result<TempDir, String> {
    log::info!("Extract audio task start: {}", file.display());
    let output_path = file.with_extension("wav");
    let output_dir = output_path
        .parent()
        .ok_or_else(|| format!("Output path has no parent: {}", output_path.display()))?;
    std::fs::create_dir_all(output_dir)
        .map_err(|error| format!("Failed to create output directory: {error}"))?;

    let directory = tempdir_in(output_dir)
        .map_err(|error| format!("Failed to create chunk directory: {error}"))?;
    let stem = output_path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Invalid output file name: {}", output_path.display()))?;
    let decoded = decode_audio(
        file,
        AudioDecodeOptions {
            sample_rate: TRANSCRIPTION_SAMPLE_RATE,
        },
        WavChunkSink {
            current: None,
            directory,
            stem: stem.to_string(),
            index: 0,
            samples_in_chunk: 0,
        },
    )
    .await?;
    if !decoded.has_audio {
        return Err("Audio chunk extraction failed: input has no audio stream".into());
    }
    log::info!("Extracted {} native WAV chunks", decoded.sink.index);
    Ok(decoded.sink.directory)
}

/// Extract the full audio track as a single 16 kHz mono WAV file.
pub async fn extract_full_audio(file: &Path) -> Result<PathBuf, String> {
    log::info!("Extract full audio: {}", file.display());
    let output_path = file.with_extension("full.wav");
    write_native_wav(file, &output_path, None).await?;
    Ok(output_path)
}

/// Extract a time segment as a 16 kHz mono WAV file.
pub(crate) async fn extract_audio_segment(
    file: &Path,
    start_sec: f64,
    duration_sec: f64,
    output_path: &Path,
) -> Result<(), String> {
    write_native_wav(
        file,
        output_path,
        Some(AudioRange {
            start: start_sec,
            duration: duration_sec,
        }),
    )
    .await
}

/// The worker owns both handles until decoding and WAV finalization finish.
/// On cancellation its temporary output is removed after the writer closes.
struct WavSink {
    writer: Option<hound::WavWriter<BufWriter<File>>>,
    temporary: NamedTempFile,
}

impl WavSink {
    fn new(directory: &Path) -> Result<Self, String> {
        let temporary = Builder::new()
            .prefix(".bili-audio-")
            .suffix(".wav")
            .tempfile_in(directory)
            .map_err(|error| format!("Failed to create temporary WAV: {error}"))?;
        let file = temporary
            .reopen()
            .map_err(|error| format!("Failed to open temporary WAV: {error}"))?;
        let writer = hound::WavWriter::new(
            BufWriter::new(file),
            hound::WavSpec {
                channels: 1,
                sample_rate: TRANSCRIPTION_SAMPLE_RATE,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .map_err(|error| format!("Failed to write WAV header: {error}"))?;
        Ok(Self {
            writer: Some(writer),
            temporary,
        })
    }

    fn publish(self, target: &Path) -> Result<(), String> {
        if self.writer.is_some() {
            return Err("Cannot publish an unfinished WAV".into());
        }
        self.temporary
            .persist(target)
            .map_err(|error| format!("Failed to store WAV {}: {error}", target.display()))?;
        Ok(())
    }
}

impl AudioSink for WavSink {
    fn write(&mut self, samples: &[i16]) -> Result<(), String> {
        let writer = self.writer.as_mut().ok_or("WAV is already finalized")?;
        for &sample in samples {
            writer
                .write_sample(sample)
                .map_err(|error| format!("Failed to write WAV samples: {error}"))?;
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), String> {
        if let Some(writer) = self.writer.take() {
            writer
                .finalize()
                .map_err(|error| format!("Failed to finalize WAV: {error}"))?;
        }
        Ok(())
    }
}

async fn write_native_wav(
    file: &Path,
    output: &Path,
    range: Option<AudioRange>,
) -> Result<(), String> {
    let directory = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let options = AudioDecodeOptions {
        sample_rate: TRANSCRIPTION_SAMPLE_RATE,
    };
    let sink = WavSink::new(directory)?;
    let decoded = match range {
        Some(range) => decode_audio_range(file, options, range, sink).await?,
        None => decode_audio(file, options, sink).await?,
    };
    if !decoded.has_audio {
        return Err("WAV extraction failed: input has no audio stream".into());
    }
    decoded.sink.publish(output)
}

/// Its open writer drops before the directory, including if the async caller
/// disappears while the worker is writing. Completed chunks stay guarded too.
struct WavChunkSink {
    current: Option<WavSink>,
    directory: TempDir,
    stem: String,
    index: usize,
    samples_in_chunk: usize,
}

impl WavChunkSink {
    fn publish_current(&mut self) -> Result<(), String> {
        if let Some(mut current) = self.current.take() {
            current.finish()?;
            current.publish(
                &self
                    .directory
                    .path()
                    .join(format!("{}_{:03}.wav", self.stem, self.index)),
            )?;
            self.index += 1;
            self.samples_in_chunk = 0;
        }
        Ok(())
    }
}

impl AudioSink for WavChunkSink {
    fn write(&mut self, mut samples: &[i16]) -> Result<(), String> {
        while !samples.is_empty() {
            if self.current.is_none() {
                self.current = Some(WavSink::new(self.directory.path())?);
            }
            let count = samples.len().min(CHUNK_SAMPLES - self.samples_in_chunk);
            self.current
                .as_mut()
                .ok_or("Missing WAV chunk writer")?
                .write(&samples[..count])?;
            self.samples_in_chunk += count;
            samples = &samples[count..];
            if self.samples_in_chunk == CHUNK_SAMPLES {
                self.publish_current()?;
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), String> {
        self.publish_current()
    }
}

#[cfg(test)]
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
#[cfg(test)]
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
    encode_flac_preview(
        file,
        output_path,
        timing,
        AudioRange { start, duration },
        pre_roll,
    )
    .await
}

/// Get the container duration in whole seconds, rounding up for audio chunks.
#[cfg(test)]
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

#[cfg(test)]
mod native_wav_tests {
    use super::*;

    fn write_pcm(path: &Path, rate: u32, channels: u16, samples: impl IntoIterator<Item = i16>) {
        let mut writer = hound::WavWriter::create(
            path,
            hound::WavSpec {
                channels,
                sample_rate: rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for sample in samples {
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
    }

    fn read_wav(path: &Path) -> Vec<i16> {
        let reader = hound::WavReader::open(path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, TRANSCRIPTION_SAMPLE_RATE);
        assert_eq!(spec.bits_per_sample, 16);
        assert_eq!(spec.sample_format, hound::SampleFormat::Int);
        reader
            .into_samples::<i16>()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn assert_no_partial_wavs(directory: &Path) {
        assert!(std::fs::read_dir(directory).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".bili-audio-")));
    }

    #[tokio::test]
    async fn native_wav_chunks_join_to_the_complete_audio_without_losing_boundary_samples() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.wav");
        let expected = (0..CHUNK_SAMPLES + 11)
            .map(|index| (index % 65_536) as u16 as i16)
            .collect::<Vec<_>>();
        write_pcm(
            &source,
            TRANSCRIPTION_SAMPLE_RATE,
            1,
            expected.iter().copied(),
        );

        let full = extract_full_audio(&source).await.unwrap();
        assert_eq!(read_wav(&full), expected);
        let chunks = extract_audio_chunks_guarded(&source).await.unwrap();
        let chunk_dir = chunks.path().to_path_buf();
        let mut files = std::fs::read_dir(&chunk_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        files.sort();
        assert_eq!(files.len(), 2);
        let first = read_wav(&files[0]);
        let last = read_wav(&files[1]);
        assert_eq!(first.len(), CHUNK_SAMPLES);
        assert_eq!(last.len(), 11);
        assert_eq!([first, last].concat(), expected);
        drop(chunks);
        assert!(!chunk_dir.exists());

        assert_no_partial_wavs(directory.path());
    }

    #[tokio::test]
    async fn native_wav_stereo_resampling_preserves_the_tail_and_format() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("stereo.wav");
        let frames = 44_113;
        write_pcm(
            &source,
            44_100,
            2,
            (0..frames).flat_map(|_| [16_384, -16_384]),
        );
        let output = extract_full_audio(&source).await.unwrap();
        let samples = read_wav(&output);
        let expected =
            (frames as f64 * f64::from(TRANSCRIPTION_SAMPLE_RATE) / 44_100.0).ceil() as usize;
        assert!(
            samples.len().abs_diff(expected) <= 1,
            "{} vs {expected}",
            samples.len()
        );
        assert!(samples.iter().all(|sample| *sample == 0));
        assert_no_partial_wavs(directory.path());
    }

    #[tokio::test]
    async fn native_wav_failures_leave_existing_output_and_remove_temporary_files() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("kept.wav");
        std::fs::write(&output, b"previous output").unwrap();
        assert!(
            write_native_wav(&directory.path().join("missing.wav"), &output, None)
                .await
                .is_err()
        );
        let no_audio = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/video/test.mp4");
        assert!(write_native_wav(&no_audio, &output, None).await.is_err());
        assert_eq!(std::fs::read(&output).unwrap(), b"previous output");
        assert_no_partial_wavs(directory.path());
    }

    #[tokio::test]
    async fn native_segment_keeps_pcm_boundaries_and_does_not_publish_failed_ranges() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.wav");
        let samples = (0..32_003)
            .map(|index| (index * 977) as u16 as i16)
            .collect::<Vec<_>>();
        write_pcm(
            &source,
            TRANSCRIPTION_SAMPLE_RATE,
            1,
            samples.iter().copied(),
        );
        let output = directory.path().join("range.wav");
        std::fs::write(&output, b"previous output").unwrap();
        extract_audio_segment(&source, 0.125, 0.777, &output)
            .await
            .unwrap();
        let decoded = read_wav(&output);
        assert_eq!(decoded.len(), 12_432);
        assert!(decoded.iter().eq(samples[2_000..14_432].iter()));
        extract_audio_segment(&source, 5.0, 0.5, &output)
            .await
            .unwrap();
        assert!(read_wav(&output).is_empty());
        extract_audio_segment(&source, 0.0, 0.0, &output)
            .await
            .unwrap();
        assert!(read_wav(&output).is_empty());
        let kept = std::fs::read(&output).unwrap();
        for (start, duration) in [(-1.0, 0.5), (0.0, -1.0), (f64::NAN, 1.0)] {
            assert!(extract_audio_segment(&source, start, duration, &output)
                .await
                .is_err());
            assert_eq!(std::fs::read(&output).unwrap(), kept);
            assert_no_partial_wavs(directory.path());
        }
        let no_audio = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/video/test.mp4");
        assert!(extract_audio_segment(&no_audio, 0.0, 1.0, &output)
            .await
            .is_err());
        assert_eq!(std::fs::read(&output).unwrap(), kept);
        assert_no_partial_wavs(directory.path());
    }

    #[tokio::test]
    async fn cancelling_native_segment_cleans_its_temporary_wav_and_keeps_the_target() {
        use std::time::{Duration, Instant};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        struct AbortOnDrop(tokio::task::AbortHandle);
        impl Drop for AbortOnDrop {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("kept.wav");
        std::fs::write(&output, b"previous output").unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let mut server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut bytes = [0; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = socket.read(&mut bytes).await.unwrap();
                assert!(read > 0 && request.len() < 16 * 1024);
                request.extend_from_slice(&bytes[..read]);
            }
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: 1000000000\r\nConnection: close\r\n\r\n").await.unwrap();
            let _ = ready_tx.send(());
            socket.read(&mut bytes).await
        });
        let _server_guard = AbortOnDrop(server.abort_handle());
        let url = PathBuf::from(format!("http://{address}/stalled.wav"));
        let target = output.clone();
        let task =
            tokio::spawn(async move { extract_audio_segment(&url, 0.0, 0.777, &target).await });
        let _task_guard = AbortOnDrop(task.abort_handle());
        tokio::time::timeout(Duration::from_secs(5), ready_rx)
            .await
            .unwrap()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!task.is_finished());
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), &mut server)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            0
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let partial = std::fs::read_dir(directory.path()).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".bili-audio-")
            });
            if !partial {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "cancelled worker retained its temporary WAV"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(std::fs::read(&output).unwrap(), b"previous output");
        assert_no_partial_wavs(directory.path());
    }
}

#[cfg(test)]
mod native_opus_tests {
    use super::*;

    #[derive(Default)]
    struct Count(u64);

    impl AudioSink for Count {
        fn write(&mut self, samples: &[i16]) -> Result<(), String> {
            self.0 += samples.len() as u64;
            Ok(())
        }
    }

    fn assert_no_partial_opus(directory: &Path) {
        assert!(std::fs::read_dir(directory).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".opus-")));
    }

    #[tokio::test]
    async fn material_audio_sample_is_a_complete_native_mono_opus_sidecar() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("素材 样本.wav");
        let mut writer = hound::WavWriter::create(
            &source,
            hound::WavSpec {
                channels: 2,
                sample_rate: 48_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for index in 0..48_000 {
            let sample = (12_000.0 * (index as f64 * 0.031).sin()) as i16;
            writer.write_sample(sample).unwrap();
            writer.write_sample(sample / 2).unwrap();
        }
        writer.finalize().unwrap();
        let expected = source.with_extension("opus");
        std::fs::write(&expected, b"old sidecar").unwrap();
        let output = extract_audio_sample(&source).await.unwrap();
        assert_eq!(output, expected);
        let data = std::fs::read(&output).unwrap();
        assert_eq!(&data[..4], b"OggS");
        let head = data
            .windows(8)
            .position(|part| part == b"OpusHead")
            .unwrap();
        assert_eq!(data[head + 9], 1);
        assert_eq!(
            u32::from_le_bytes(data[head + 12..head + 16].try_into().unwrap()),
            16_000
        );
        let decoded = decode_audio(
            &output,
            AudioDecodeOptions {
                sample_rate: 48_000,
            },
            Count::default(),
        )
        .await
        .unwrap();
        assert!(decoded.has_audio);
        assert_eq!(decoded.samples, 48_000);
        assert_eq!(decoded.sink.0, decoded.samples);
        let original = std::fs::read(&output).unwrap();
        assert!(extract_audio_sample(&output).await.is_err());
        assert_eq!(std::fs::read(&output).unwrap(), original);
        assert_no_partial_opus(directory.path());
    }

    #[tokio::test]
    async fn material_audio_sample_failures_preserve_previous_sidecars() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.wav");
        let invalid = directory.path().join("invalid.wav");
        std::fs::write(&invalid, b"invalid media").unwrap();
        let video = directory.path().join("video.mp4");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/video/test.mp4"),
            &video,
        )
        .unwrap();
        for source in [&missing, &invalid, &video] {
            let output = source.with_extension("opus");
            std::fs::write(&output, b"previous sidecar").unwrap();
            assert!(extract_audio_sample(source).await.is_err());
            assert_eq!(std::fs::read(&output).unwrap(), b"previous sidecar");
            assert_no_partial_opus(directory.path());
        }
    }
}
