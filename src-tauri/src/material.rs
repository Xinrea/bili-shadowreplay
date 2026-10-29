//! Files that belong to a material (an entry of the `videos` table) and share
//! its file stem: thumbnail, waveform sample, cached subtitles, the danmaku cut
//! from its recording and waveform peaks.

use std::path::{Path, PathBuf};

use ffmpeg_utils::{decode_audio, AudioDecodeOptions, AudioSink};
use recorder::danmu::DanmuEntry;
use serde::{Deserialize, Serialize};

/// Waveform resolution of [`Peaks`].
pub const PEAKS_PER_SECOND: u32 = 50;
const PEAKS_SAMPLE_RATE: u32 = 8000;
const PEAKS_VERSION: u32 = 1;

pub fn danmu_path(video: &Path) -> PathBuf {
    video.with_extension("danmu.json")
}

pub fn peaks_path(video: &Path) -> PathBuf {
    video.with_extension("peaks.json")
}

/// Sidecar files to remove together with the material.
pub fn sidecar_paths(video: &Path) -> Vec<PathBuf> {
    ["srt", "wav", "mp3", "opus", "danmu.json", "peaks.json"]
        .iter()
        .map(|extension| video.with_extension(extension))
        .collect()
}

/// Save danmaku with `ts` in milliseconds from the start of the material.
pub async fn save_danmus(video: &Path, danmus: &[DanmuEntry]) -> Result<(), String> {
    let path = danmu_path(video);
    let content = serde_json::to_vec(danmus).map_err(|e| e.to_string())?;
    tokio::fs::write(&path, content)
        .await
        .map_err(|e| format!("写入素材弹幕失败: {} ({e})", path.display()))
}

/// Danmaku saved with the material; empty when it has none.
pub async fn load_danmus(video: &Path) -> Vec<DanmuEntry> {
    let path = danmu_path(video);
    let Ok(content) = tokio::fs::read(&path).await else {
        return Vec::new();
    };
    serde_json::from_slice(&content).unwrap_or_else(|e| {
        log::warn!("Invalid material danmaku {}: {e}", path.display());
        Vec::new()
    })
}

/// Peak amplitude (0-255) of each 1/[`PEAKS_PER_SECOND`] s of audio.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peaks {
    pub version: u32,
    pub rate: u32,
    pub peaks: Vec<u8>,
}

/// Waveform peaks of the material, computed once and cached next to it.
pub async fn peaks(video: &Path) -> Result<Peaks, String> {
    let cache = peaks_path(video);
    if let Ok(content) = tokio::fs::read(&cache).await {
        if let Ok(peaks) = serde_json::from_slice::<Peaks>(&content) {
            if peaks.version == PEAKS_VERSION && peaks.rate == PEAKS_PER_SECOND {
                return Ok(peaks);
            }
        }
    }
    let peaks = compute_peaks(video).await?;
    if let Ok(content) = serde_json::to_vec(&peaks) {
        if let Err(e) = tokio::fs::write(&cache, content).await {
            log::warn!("Failed to cache waveform peaks {}: {e}", cache.display());
        }
    }
    Ok(peaks)
}

#[derive(Default)]
struct PeakSink {
    peaks: Vec<u8>,
    current: u16,
    samples: usize,
}

impl PeakSink {
    fn flush_window(&mut self) {
        self.peaks
            .push((u32::from(self.current) * 255 / 32768) as u8);
        self.current = 0;
        self.samples = 0;
    }
}

impl AudioSink for PeakSink {
    fn write(&mut self, samples: &[i16]) -> Result<(), String> {
        let window = (PEAKS_SAMPLE_RATE / PEAKS_PER_SECOND) as usize;
        for &sample in samples {
            self.current = self.current.max(sample.unsigned_abs());
            self.samples += 1;
            if self.samples == window {
                self.flush_window();
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), String> {
        if self.samples > 0 {
            self.flush_window();
        }
        Ok(())
    }
}

async fn compute_peaks(video: &Path) -> Result<Peaks, String> {
    let decoded = decode_audio(
        video,
        AudioDecodeOptions {
            sample_rate: PEAKS_SAMPLE_RATE,
        },
        PeakSink::default(),
    )
    .await?;
    Ok(Peaks {
        version: PEAKS_VERSION,
        rate: PEAKS_PER_SECOND,
        peaks: decoded.sink.peaks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ffmpeg_utils::ffmpeg_command;

    #[tokio::test]
    async fn native_peaks_keep_full_scale_partial_windows_and_cached_results() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("peaks.wav");
        let mut writer = hound::WavWriter::create(
            &source,
            hound::WavSpec {
                channels: 1,
                sample_rate: PEAKS_SAMPLE_RATE,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for sample in std::iter::repeat_n(i16::MIN, 160)
            .chain(std::iter::repeat_n(16_384, 160))
            .chain(std::iter::repeat_n(8_192, 7))
        {
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();

        let computed = peaks(&source).await.unwrap();
        assert_eq!(computed.rate, 50);
        assert_eq!(computed.peaks, [255, 127, 63]);
        assert!(peaks_path(&source).is_file());
        std::fs::remove_file(&source).unwrap();
        assert_eq!(peaks(&source).await.unwrap().peaks, computed.peaks);
    }

    #[tokio::test]
    async fn native_peaks_without_an_audio_track_are_empty() {
        let silent = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/video/test.mp4");
        assert!(compute_peaks(&silent).await.unwrap().peaks.is_empty());
    }

    async fn run_ffmpeg(args: &[&str], output: &Path) {
        let result = ffmpeg_command()
            .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
            .args(args)
            .arg("-y")
            .arg(output)
            .output()
            .await
            .unwrap();
        assert!(result.status.success());
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg"]
    async fn ffmpeg_peaks_are_cached_and_silent_videos_are_flat() {
        let directory = tempfile::tempdir().unwrap();
        let tone = directory.path().join("tone.mp4");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=64x64:r=10:d=2",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-c:a",
                "aac",
                "-shortest",
            ],
            &tone,
        )
        .await;
        let first = peaks(&tone).await.unwrap();
        assert!(
            (first.peaks.len() as i64 - 100).abs() <= 3,
            "{}",
            first.peaks.len()
        );
        assert!(first.peaks[50] > 0);
        assert!(peaks_path(&tone).exists());
        assert_eq!(peaks(&tone).await.unwrap().peaks, first.peaks);

        let silent = directory.path().join("silent.mp4");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=64x64:r=10:d=1",
                "-c:v",
                "libx264",
            ],
            &silent,
        )
        .await;
        assert!(peaks(&silent).await.unwrap().peaks.is_empty());
    }

    #[tokio::test]
    async fn danmaku_round_trips_and_missing_files_are_empty() {
        let directory = tempfile::tempdir().unwrap();
        let video = directory.path().join("clip.mp4");
        assert!(load_danmus(&video).await.is_empty());
        let danmus = vec![DanmuEntry {
            ts: 1500,
            event_type: "danmu".into(),
            content: "hi".into(),
            emoji_content: None,
            user_name: Some("user".into()),
            price: None,
            sc_duration: None,
        }];
        save_danmus(&video, &danmus).await.unwrap();
        assert_eq!(danmu_path(&video), directory.path().join("clip.danmu.json"));
        let loaded = load_danmus(&video).await;
        assert_eq!(loaded.len(), 1);
        assert_eq!((loaded[0].ts, loaded[0].content.as_str()), (1500, "hi"));
    }
}
