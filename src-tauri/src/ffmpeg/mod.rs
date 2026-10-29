use std::fmt;
use std::path::Path;

pub mod audio;
pub mod clip;
mod danmu;
pub mod general;
pub mod hwaccel;
pub mod metadata;
pub mod native_overlay;
pub mod playlist;
pub mod project;
mod project_plan;
pub mod runner;
pub mod subtitle;
mod subtitle_native;

#[cfg(test)]
pub(crate) use audio::get_audio_duration;
pub use audio::{extract_audio_sample, extract_full_audio};
pub use clip::{clip_from_video_file, transcode, trim_video};
pub use danmu::{encode_video_danmu, encode_video_danmu_with_settings};
pub use metadata::{
    check_ffmpeg, check_videos, extract_frame, generate_thumbnail, get_random_thumbnail_timestamp,
};
pub use subtitle::{encode_video_subtitle, generate_video_subtitle};

#[cfg(test)]
use crate::constants;
use crate::progress::progress_reporter::{ProgressReporter, ProgressReporterTrait};
#[cfg(test)]
use crate::subtitle_generator::SubtitleGeneratorType;
#[cfg(test)]
use ffmpeg_utils::extract_video_metadata;
use serde::{Deserialize, Serialize};

use self::runner::{run_native_media, ProgressMode, ProgressTotal};

/// File name of `path` as UTF-8, for display and ffmpeg filter strings.
#[cfg(test)]
fn file_name_str(path: &Path) -> Result<&str, String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Invalid file name: {}", path.display()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Range {
    pub start: f64,
    pub end: f64,
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}, {}]", self.start, self.end)
    }
}

impl Range {
    pub fn duration(&self) -> f64 {
        self.end - self.start
    }

    pub fn is_in(&self, v: f64) -> bool {
        v >= self.start && v <= self.end
    }
}

/// Duration of each `xfade` / `acrossfade` overlap when concatenating clips.
pub const TRANSITION_DURATION_SECS: f64 = 1.0;

pub fn applies_clip_transition(transition: Option<&str>, clip_count: usize) -> bool {
    clip_count >= 2 && matches!(transition, Some(t) if t != "none")
}

/// Start timestamps (milliseconds) of each clip on the concatenated timeline.
///
/// Video transitions overlap adjacent clips by [`TRANSITION_DURATION_SECS`], so
/// later danmaku must use the same shortened timeline rather than a simple sum.
pub fn clip_timeline_anchors(ranges: &[Range], transition: Option<&str>) -> Vec<i64> {
    let overlap_ms = if applies_clip_transition(transition, ranges.len()) {
        (TRANSITION_DURATION_SECS * 1000.0) as i64
    } else {
        0
    };
    let mut anchors = vec![0i64; ranges.len()];
    for i in 1..ranges.len() {
        anchors[i] = (ranges[i - 1].duration() * 1000.0) as i64 + anchors[i - 1] - overlap_ms;
    }
    anchors
}

// 执行FFmpeg转换的通用函数
async fn execute_native_conversion(
    plan: ffmpeg_utils::MediaPlan,
    reporter: &ProgressReporter,
    mode_name: &str,
) -> Result<(), String> {
    run_native_media(
        plan,
        ProgressMode::stage(
            format!("转换视频格式（{mode_name}）"),
            ProgressTotal::FirstInput,
        ),
        format!("视频格式转换失败 ({mode_name})"),
        Some(reporter),
    )
    .await?;

    reporter
        .update(&format!("视频格式转换完成 100% ({mode_name})"))
        .await;
    Ok(())
}

// 尝试流复制转换（无损，速度快）
pub async fn try_stream_copy_conversion(
    source: &Path,
    dest: &Path,
    reporter: &ProgressReporter,
) -> Result<(), String> {
    reporter.update("正在转换视频格式... 0% (无损模式)").await;

    let mut plan = clip::single_input_plan(
        source,
        dest,
        ffmpeg_utils::StreamMode::Copy,
        ffmpeg_utils::StreamMode::Copy,
    );
    plan.outputs[0].negative_ts = ffmpeg_utils::NegativeTs::MakeZero;
    plan.outputs[0].faststart = true;
    execute_native_conversion(plan, reporter, "无损转换").await
}

// 高质量重编码转换（兼容性好，质量高）
pub async fn try_high_quality_conversion(
    source: &Path,
    dest: &Path,
    reporter: &ProgressReporter,
) -> Result<(), String> {
    reporter.update("正在转换视频格式... 0% (高质量模式)").await;

    let video = ffmpeg_utils::VideoEncodeOptions {
        encoder: "libx264".into(),
        options: [
            ("preset".into(), "slow".into()),
            ("crf".into(), "18".into()),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    let audio = ffmpeg_utils::AudioEncodeOptions {
        bit_rate: Some(192_000),
        ..Default::default()
    };
    let mut plan = clip::single_input_plan(
        source,
        dest,
        ffmpeg_utils::StreamMode::Video(video),
        ffmpeg_utils::StreamMode::Audio(audio),
    );
    plan.outputs[0].negative_ts = ffmpeg_utils::NegativeTs::MakeZero;
    plan.outputs[0].faststart = true;
    execute_native_conversion(plan, reporter, "高质量转换").await
}

// 带进度的视频格式转换函数（智能质量保持策略）
pub async fn convert_video_format(
    source: &Path,
    dest: &Path,
    reporter: &ProgressReporter,
) -> Result<(), String> {
    // 先尝试stream copy（无损转换），如果失败则使用高质量重编码
    match try_stream_copy_conversion(source, dest, reporter).await {
        Ok(()) => Ok(()),
        Err(stream_copy_error) => {
            reporter.update("流复制失败，使用高质量重编码模式...").await;
            log::warn!("Stream copy failed: {stream_copy_error}, falling back to re-encoding");
            try_high_quality_conversion(source, dest, reporter).await
        }
    }
}

// tests
#[cfg(test)]
mod tests {
    use super::*;

    // 测试 Range 结构体
    #[test]
    fn test_range_creation() {
        let range = Range {
            start: 10.0,
            end: 30.0,
        };
        assert_eq!(range.start, 10.0);
        assert_eq!(range.end, 30.0);
        assert_eq!(range.duration(), 20.0);
    }

    #[test]
    fn test_range_duration() {
        let range = Range {
            start: 0.0,
            end: 60.0,
        };
        assert_eq!(range.duration(), 60.0);

        let range2 = Range {
            start: 15.5,
            end: 45.5,
        };
        assert_eq!(range2.duration(), 30.0);
    }

    #[test]
    fn test_range_display() {
        let range = Range {
            start: 5.0,
            end: 25.0,
        };
        assert_eq!(range.to_string(), "[5, 25]");
    }

    #[test]
    fn test_range_edge_cases() {
        let zero_range = Range {
            start: 0.0,
            end: 0.0,
        };
        assert_eq!(zero_range.duration(), 0.0);

        let negative_start = Range {
            start: -5.0,
            end: 10.0,
        };
        assert_eq!(negative_start.duration(), 15.0);

        let large_range = Range {
            start: 1000.0,
            end: 2000.0,
        };
        assert_eq!(large_range.duration(), 1000.0);
    }

    fn sample_ranges() -> Vec<Range> {
        vec![
            Range {
                start: 0.0,
                end: 10.0,
            },
            Range {
                start: 50.0,
                end: 60.0,
            },
            Range {
                start: 100.0,
                end: 110.0,
            },
        ]
    }

    #[test]
    fn clip_timeline_anchors_without_transition_sum_durations() {
        assert_eq!(
            clip_timeline_anchors(&sample_ranges(), None),
            vec![0, 10_000, 20_000]
        );
        assert_eq!(
            clip_timeline_anchors(&sample_ranges(), Some("none")),
            vec![0, 10_000, 20_000]
        );
    }

    #[test]
    fn clip_timeline_anchors_with_transition_subtract_xfade_overlap() {
        // 三段各 10s，转场各重叠 1s：0 / 9s / 18s
        assert_eq!(
            clip_timeline_anchors(&sample_ranges(), Some("fade")),
            vec![0, 9_000, 18_000]
        );
        assert_eq!(
            clip_timeline_anchors(&sample_ranges(), Some("dissolve")),
            vec![0, 9_000, 18_000]
        );
    }

    #[test]
    fn clip_timeline_anchors_single_range_stays_at_zero_with_transition() {
        let ranges = vec![Range {
            start: 5.0,
            end: 15.0,
        }];
        assert_eq!(clip_timeline_anchors(&ranges, Some("fade")), vec![0]);
    }

    #[test]
    fn clip_timeline_anchors_empty_ranges() {
        assert!(clip_timeline_anchors(&[], Some("fade")).is_empty());
    }

    #[derive(Clone)]
    struct NoopReporter;

    #[async_trait::async_trait]
    impl ProgressReporterTrait for NoopReporter {
        async fn update(&self, _content: &str) {}
        async fn finish(&self, _success: bool, _message: &str) {}
    }

    // 字幕缺失/为空时应提前失败，而不是产出没有字幕的视频
    #[tokio::test]
    async fn test_encode_video_subtitle_rejects_missing_subtitle() {
        let result = encode_video_subtitle(
            &NoopReporter,
            Path::new("tests/video/test.mp4"),
            Path::new("tests/video/does_not_exist.srt"),
            "FontSize=24".to_string(),
            None,
        )
        .await;

        let err = result.expect_err("缺失字幕应当返回 Err");
        assert!(err.contains("字幕文件不可用"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn test_encode_video_subtitle_rejects_empty_subtitle() {
        let empty_srt = std::env::temp_dir().join("bsr_empty_subtitle_test.srt");
        std::fs::write(&empty_srt, "").expect("write empty srt");

        let result = encode_video_subtitle(
            &NoopReporter,
            Path::new("tests/video/test.mp4"),
            &empty_srt,
            "FontSize=24".to_string(),
            None,
        )
        .await;

        let _ = std::fs::remove_file(&empty_srt);

        let err = result.expect_err("空字幕应当返回 Err");
        assert!(err.contains("字幕文件为空"), "unexpected error: {err}");
    }

    // 测试视频元数据提取
    #[tokio::test]
    async fn test_extract_video_metadata() {
        let test_video = Path::new("tests/video/test.mp4");
        if test_video.exists() {
            let metadata = extract_video_metadata(test_video).await.unwrap();
            println!("metadata: {:?}", metadata);
            assert!(metadata.duration > 0.0);
            assert!(metadata.width > 0);
            assert!(metadata.height > 0);
        }
    }

    // 测试音频时长获取
    #[tokio::test]
    async fn test_get_audio_duration() {
        let test_audio = Path::new("tests/audio/test.wav");
        if test_audio.exists() {
            let duration = get_audio_duration(test_audio).await.unwrap();
            assert!(duration > 0);
        }
    }

    // 测试缩略图生成
    #[tokio::test]
    async fn test_generate_thumbnail() {
        let file = Path::new("tests/video/test.mp4");
        if file.exists() {
            let thumbnail_file = generate_thumbnail(file, 0.0).await.unwrap();
            assert!(thumbnail_file.exists());
            assert_eq!(thumbnail_file.extension().unwrap(), "jpg");
            // clean up
            let _ = std::fs::remove_file(thumbnail_file);
        }
    }

    // 测试 FFmpeg 版本检查
    #[tokio::test]
    async fn test_check_ffmpeg() {
        let result = check_ffmpeg().await;
        match result {
            Ok(version) => {
                assert!(!version.is_empty());
                // FFmpeg 版本字符串可能不包含 "ffmpeg" 这个词，所以检查是否包含数字
                assert!(version.chars().any(|c| c.is_ascii_digit()));
            }
            Err(_) => {
                // FFmpeg 可能没有安装，这是正常的
                println!("FFmpeg not available for testing");
            }
        }
    }

    // 测试硬件加速能力探测
    #[tokio::test]
    async fn test_list_supported_hwaccels() {
        match super::hwaccel::list_supported_hwaccels().await {
            Ok(hwaccels) => {
                println!("hwaccels: {:?}", hwaccels);
                let mut sorted = hwaccels.clone();
                sorted.sort();
                sorted.dedup();
                assert_eq!(sorted.len(), hwaccels.len());
            }
            Err(_) => {
                println!("FFmpeg hardware acceleration query not available for testing");
            }
        }
    }

    // 测试字幕生成错误处理
    #[tokio::test]
    async fn test_generate_video_subtitle_errors() {
        let test_file = Path::new("tests/video/test.mp4");

        // 测试 Whisper 类型 - 模型未配置
        let result = generate_video_subtitle(
            None,
            test_file,
            SubtitleGeneratorType::Whisper,
            Path::new(""),
            "",
            "",
            "",
            "",
            "zh",
            None,
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Whisper model not configured"));

        // 测试 Whisper Online 类型 - API key 未配置
        let result = generate_video_subtitle(
            None,
            test_file,
            SubtitleGeneratorType::WhisperOnline,
            Path::new(""),
            "",
            "",
            "",
            "",
            "zh",
            None,
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("API key not configured"));

        let result = generate_video_subtitle(
            None,
            test_file,
            SubtitleGeneratorType::PowerLive,
            Path::new(""),
            "",
            "",
            "",
            "",
            "zh",
            None,
        )
        .await;
        assert!(result.unwrap_err().contains("登录 PowerLive"));

        // 未知类型应在服务边界之前被拒绝
        let error = SubtitleGeneratorType::parse("unknown_type").unwrap_err();
        assert!(error.contains("Unknown subtitle generator type"));
    }

    // 测试文件名和路径处理
    #[test]
    fn test_filename_processing() {
        let test_file = Path::new("tests/video/test.mp4");

        // 测试字幕文件名生成
        let subtitle_filename = format!(
            "{}{}",
            constants::PREFIX_SUBTITLE,
            file_name_str(test_file).expect("test path is valid UTF-8")
        );
        assert!(subtitle_filename.starts_with(constants::PREFIX_SUBTITLE));
        assert!(subtitle_filename.contains("test.mp4"));

        // 测试弹幕文件名生成
        let danmu_filename = format!(
            "{}{}",
            constants::PREFIX_DANMAKU,
            file_name_str(test_file).expect("test path is valid UTF-8")
        );
        assert!(danmu_filename.starts_with(constants::PREFIX_DANMAKU));
        assert!(danmu_filename.contains("test.mp4"));
    }

    // 测试音频分块目录结构
    #[test]
    fn test_audio_chunk_directory_structure() {
        let test_file = Path::new("tests/audio/test.wav");
        let output_path = test_file.with_extension("wav");
        let output_dir = output_path.parent().unwrap();
        let base_name = output_path.file_stem().unwrap().to_str().unwrap();
        let chunk_dir = output_dir.join(format!("{base_name}_chunks"));

        assert!(chunk_dir.to_string_lossy().contains("_chunks"));
        assert!(chunk_dir.to_string_lossy().contains("test"));
    }

    #[test]
    fn test_range_is_in_inside() {
        let r = Range {
            start: 1.0,
            end: 5.0,
        };
        assert!(r.is_in(3.0));
    }

    #[test]
    fn test_range_is_in_at_boundaries() {
        let r = Range {
            start: 1.0,
            end: 5.0,
        };
        assert!(r.is_in(1.0));
        assert!(r.is_in(5.0));
    }

    #[test]
    fn test_range_is_in_outside() {
        let r = Range {
            start: 1.0,
            end: 5.0,
        };
        assert!(!r.is_in(0.9));
        assert!(!r.is_in(5.1));
    }
}
