use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::command::{ffprobe_command, path_str};

/// Basic information about a media file, as reported by `ffprobe`.
///
/// This is the single description of a video shared by the application (archive
/// length, clip compatibility, agent tool results) and the recorder (segment
/// validation), so both layers agree on what a file is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoMetadata {
    /// Length of the video track in seconds. A live HLS segment may not report
    /// a duration on its video track; there the span of the container is used
    /// instead. `0.0` when neither is available.
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub video_codec: String,
    /// Empty when the file has no audio stream.
    pub audio_codec: String,
    /// Container bit rate in bits per second, `0` when unknown.
    pub bitrate: u64,
    /// Frame rate of the video track, `0.0` when unknown.
    pub fps: f64,
    /// Size of the file in bytes, `0` when unknown.
    pub file_size: u64,
}

impl VideoMetadata {
    /// Whether no video parameters could be read at all, which is how a Bilibili
    /// TS segment that still lacks its SPS/PPS shows up. The recorder appends
    /// such a segment behind the previous one instead of giving up on it.
    pub fn seems_corrupted(&self) -> bool {
        self.width == 0 && self.height == 0
    }

    /// Whether both files carry the same resolution and codecs.
    ///
    /// Length, frame rate and bit rate are deliberately not part of this: the
    /// recorder uses it to detect a resolution change in the middle of a stream,
    /// where per-segment differences in those values are normal.
    pub fn same_stream_shape(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.video_codec == other.video_codec
            && self.audio_codec == other.audio_codec
    }
}

/// Extract basic information from a media file with `ffprobe`.
///
/// # Arguments
/// * `file_path` - The path to the media file.
///
/// # Returns
/// A `Result` containing the video metadata or an error message.
///
/// Probing only fails when the process cannot be run, exits non-zero (the file
/// is not readable by ffprobe) or reports no stream list at all (for example a
/// bare `m4s` media segment without its initialization segment). Fields which
/// the container does not report stay at `0`/empty so that callers can inspect
/// [`VideoMetadata::seems_corrupted`] instead of matching on error strings.
pub async fn extract_video_metadata(file_path: &Path) -> Result<VideoMetadata, String> {
    let output = ffprobe_command()
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path_str(file_path)?)
        .output()
        .await
        .map_err(|e| format!("执行ffprobe失败: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "ffprobe执行失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("解析ffprobe输出失败: {e}"))?;

    parse_video_metadata(&json)
}

/// Turn `ffprobe -show_format -show_streams` output into [`VideoMetadata`].
///
/// `ffprobe` prints numeric format fields (durations, bit rates, sizes) as
/// strings, while stream fields such as width and height are JSON numbers. The
/// first stream of each kind describes the file.
fn parse_video_metadata(json: &serde_json::Value) -> Result<VideoMetadata, String> {
    let streams = json["streams"].as_array().ok_or("未找到视频流信息")?;

    if streams.is_empty() {
        return Err("未找到视频流".to_string());
    }

    let video = streams
        .iter()
        .find(|stream| stream["codec_type"].as_str() == Some("video"));
    let audio = streams
        .iter()
        .find(|stream| stream["codec_type"].as_str() == Some("audio"));
    let format = &json["format"];

    Ok(VideoMetadata {
        duration: video_duration(video, format),
        width: video
            .and_then(|stream| stream["width"].as_u64())
            .unwrap_or(0) as u32,
        height: video
            .and_then(|stream| stream["height"].as_u64())
            .unwrap_or(0) as u32,
        video_codec: codec_name(video),
        audio_codec: codec_name(audio),
        bitrate: format["bit_rate"]
            .as_str()
            .and_then(|rate| rate.parse().ok())
            .unwrap_or(0),
        fps: video
            .and_then(|stream| stream["r_frame_rate"].as_str())
            .and_then(parse_frame_rate)
            .unwrap_or(0.0),
        file_size: format["size"]
            .as_str()
            .and_then(|size| size.parse().ok())
            .unwrap_or(0),
    })
}

/// Length of the video track, falling back to the container span when the track
/// reports none or an explicit zero.
fn video_duration(video: Option<&serde_json::Value>, format: &serde_json::Value) -> f64 {
    let stream_duration = video
        .and_then(|stream| stream["duration"].as_str())
        .and_then(|duration| duration.parse::<f64>().ok());

    stream_duration
        .filter(|duration| *duration > 0.0)
        .or_else(|| {
            format["duration"]
                .as_str()
                .and_then(|duration| duration.parse::<f64>().ok())
                .filter(|duration| *duration > 0.0)
        })
        .unwrap_or(0.0)
}

fn codec_name(stream: Option<&serde_json::Value>) -> String {
    stream
        .and_then(|stream| stream["codec_name"].as_str())
        .unwrap_or_default()
        .to_owned()
}

/// Parse a `r_frame_rate` value such as `60/1`.
fn parse_frame_rate(rate: &str) -> Option<f64> {
    let (numerator, denominator) = rate.split_once('/')?;
    let numerator = numerator.parse::<f64>().ok()?;
    let denominator = denominator.parse::<f64>().ok()?;

    (denominator != 0.0).then(|| numerator / denominator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;
    use std::process::Stdio;

    /// A trimmed down `ffprobe` report of a 1080p60 H.264/AAC file.
    fn ffprobe_report() -> serde_json::Value {
        json!({
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "h264",
                    "width": 1920,
                    "height": 1080,
                    "duration": "10.000000",
                    "r_frame_rate": "60/1",
                },
                {
                    "codec_type": "audio",
                    "codec_name": "aac",
                    "duration": "10.000000",
                }
            ],
            "format": {
                "duration": "10.000000",
                "bit_rate": "6081300",
                "size": "7601625",
            }
        })
    }

    #[test]
    fn reports_every_field_the_container_provides() {
        let metadata = parse_video_metadata(&ffprobe_report()).unwrap();

        assert_eq!(metadata.duration, 10.0);
        assert_eq!(metadata.width, 1920);
        assert_eq!(metadata.height, 1080);
        assert_eq!(metadata.video_codec, "h264");
        assert_eq!(metadata.audio_codec, "aac");
        assert_eq!(metadata.bitrate, 6_081_300);
        assert_eq!(metadata.fps, 60.0);
        assert_eq!(metadata.file_size, 7_601_625);
    }

    #[test]
    fn falls_back_to_container_duration_when_the_video_track_has_none() {
        // A TS segment that reports a container span but no per-track duration.
        let mut report = ffprobe_report();
        report["streams"][0]["duration"] = json!(null);
        report["format"]["duration"] = json!("5.023222");

        assert_eq!(parse_video_metadata(&report).unwrap().duration, 5.023_222);
    }

    #[test]
    fn falls_back_to_container_duration_when_the_video_track_reports_zero() {
        // Bilibili's fMP4 initialization segment reports `0.000000` per track
        // and no container duration at all.
        let mut report = ffprobe_report();
        report["streams"][0]["duration"] = json!("0.000000");
        report["streams"][1]["duration"] = json!("0.000000");
        report["format"]["duration"] = json!("4.000000");

        assert_eq!(parse_video_metadata(&report).unwrap().duration, 4.0);

        report["format"] = json!({});
        assert_eq!(parse_video_metadata(&report).unwrap().duration, 0.0);
    }

    #[test]
    fn keeps_a_track_duration_that_is_shorter_than_the_container() {
        // The video track ends earlier than the audio: the track length is what
        // clip and archive accounting has always used.
        let mut report = ffprobe_report();
        report["streams"][0]["duration"] = json!("9.500000");
        report["format"]["duration"] = json!("10.000000");

        assert_eq!(parse_video_metadata(&report).unwrap().duration, 9.5);
    }

    #[test]
    fn a_file_without_audio_reports_an_empty_audio_codec() {
        let mut report = ffprobe_report();
        report["streams"] = json!([report["streams"][0].clone()]);

        assert_eq!(parse_video_metadata(&report).unwrap().audio_codec, "");
    }

    #[test]
    fn an_unparsable_frame_rate_is_reported_as_zero() {
        let mut report = ffprobe_report();
        report["streams"][0]["r_frame_rate"] = json!("0/0");
        assert_eq!(parse_video_metadata(&report).unwrap().fps, 0.0);

        report["streams"][0]["r_frame_rate"] = json!(null);
        assert_eq!(parse_video_metadata(&report).unwrap().fps, 0.0);
    }

    #[test]
    fn rejects_a_report_without_streams() {
        let error = parse_video_metadata(&json!({})).unwrap_err();
        assert!(
            error.contains("未找到视频流信息"),
            "unexpected error: {error}"
        );

        let error = parse_video_metadata(&json!({ "streams": [] })).unwrap_err();
        assert!(error.contains("未找到视频流"), "unexpected error: {error}");
    }

    #[test]
    fn missing_video_parameters_are_flagged_as_corrupted() {
        // ffprobe lists the stream but cannot read its parameters yet.
        let report = json!({
            "streams": [{ "codec_type": "video", "codec_name": "h264" }],
            "format": { "duration": "4.000000" }
        });

        let metadata = parse_video_metadata(&report).unwrap();

        assert!(metadata.seems_corrupted());
        assert_eq!(metadata.video_codec, "h264");
        assert!(!parse_video_metadata(&ffprobe_report())
            .unwrap()
            .seems_corrupted());
    }

    #[test]
    fn stream_shape_ignores_length_and_bitrate() {
        let metadata = parse_video_metadata(&ffprobe_report()).unwrap();

        let mut longer = metadata.clone();
        longer.duration = 20.0;
        longer.bitrate *= 2;
        longer.file_size *= 2;
        longer.fps = 30.0;
        assert!(metadata.same_stream_shape(&longer));

        let mut different_resolution = metadata.clone();
        different_resolution.width = 1280;
        different_resolution.height = 720;
        assert!(!metadata.same_stream_shape(&different_resolution));

        let mut different_video_codec = metadata.clone();
        different_video_codec.video_codec = "hevc".to_string();
        assert!(!metadata.same_stream_shape(&different_video_codec));

        let mut different_audio_codec = metadata.clone();
        different_audio_codec.audio_codec = "opus".to_string();
        assert!(!metadata.same_stream_shape(&different_audio_codec));
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/video")
            .join(name)
    }

    /// The unit tests also run in environments without ffmpeg installed (CI
    /// installs no ffmpeg packages), so the fixture based tests skip there
    /// instead of failing.
    async fn ffprobe_available() -> bool {
        ffprobe_command()
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .is_ok()
    }

    #[tokio::test]
    async fn probes_a_recorded_file() {
        let file = fixture("test.mp4");
        if !file.exists() || !ffprobe_available().await {
            return;
        }

        let metadata = extract_video_metadata(&file).await.unwrap();

        assert!(metadata.duration > 0.0);
        assert_eq!((metadata.width, metadata.height), (3240, 2160));
        assert!(metadata.video_codec.starts_with("h264"));
        assert_eq!(metadata.audio_codec, "");
        assert!(metadata.file_size > 0);
        assert!(!metadata.seems_corrupted());
    }

    #[tokio::test]
    async fn probes_a_segment_that_has_no_duration_of_its_own() {
        let file = fixture("init.m4s");
        if !file.exists() || !ffprobe_available().await {
            return;
        }

        let metadata = extract_video_metadata(&file).await.unwrap();

        // The initialization segment carries resolution and codecs but no
        // playable duration of its own.
        assert_eq!(metadata.duration, 0.0);
        assert_eq!((metadata.width, metadata.height), (1920, 1080));
        assert_eq!(metadata.video_codec, "h264");
        assert_eq!(metadata.audio_codec, "aac");
    }

    #[tokio::test]
    async fn a_media_segment_without_its_initialization_segment_is_an_error() {
        let file = fixture("segment.m4s");
        if !file.exists() || !ffprobe_available().await {
            return;
        }

        // ffprobe reads no stream list from it at all.
        assert!(extract_video_metadata(&file).await.is_err());
    }
}
