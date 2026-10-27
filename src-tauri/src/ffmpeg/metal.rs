//! macOS Skia Metal / Metal overlay / VideoToolbox export worker.

use std::{
    num::NonZeroU32,
    path::{Path, PathBuf},
    process::Stdio,
};

use super::{
    encoding::{VideoCodec, VideoEncodingSettings},
    native_overlay::OverlaySpec,
};
use crate::progress::progress_reporter::{ProgressReporterTrait, StageProgress};

/// Run the native pipeline in a child so task cancellation also stops FFmpeg
/// and releases Metal resources without blocking the application runtime.
pub(super) async fn encode_video(
    reporter: Option<&impl ProgressReporterTrait>,
    input: &Path,
    spec: OverlaySpec,
    output: &Path,
    settings: &VideoEncodingSettings,
    bitrate: Option<NonZeroU32>,
) -> Result<PathBuf, String> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    spec.layout()?;
    let output = std::path::absolute(output)
        .map_err(|e| e.to_string())?
        .with_extension("mp4");
    let directory = tempfile::Builder::new()
        .prefix(".bili-metal-")
        .tempdir_in(output.parent().ok_or("输出路径缺少目录")?)
        .map_err(|e| e.to_string())?;
    let pending_output = directory.path().join("output.mp4");
    let ass = directory.path().join("danmu.ass");
    std::fs::write(&ass, &spec.input.documents[0].text).map_err(|e| e.to_string())?;
    let images: Vec<_> = spec
        .input
        .images
        .iter()
        .map(|image| {
            serde_json::json!({
                "path": image.path, "start": image.start, "end": image.end,
                "from": {"x": image.from.0, "y": image.from.1},
                "to": {"x": image.to.0, "y": image.to.1},
                "width": image.width, "height": image.height,
                "opacity": image.opacity,
            })
        })
        .collect();
    let config = serde_json::json!({
        "source": std::fs::canonicalize(input).map_err(|e| e.to_string())?,
        "output": pending_output, "ass_file": ass, "overlays": images, "fps": spec.fps,
        "encoder": if settings.codec == VideoCodec::Hevc { "hevc_videotoolbox" } else { "h264_videotoolbox" },
        "bitrate_bps": settings.bitrate_kbps(bitrate) * 1000,
        "speed_priority": settings.videotoolbox_speed_priority,
    });
    let config_file = directory.path().join("job.json");
    std::fs::write(&config_file, config.to_string()).map_err(|e| e.to_string())?;
    let error_path = directory.path().join("worker.log");
    let error_file = std::fs::File::create(&error_path).map_err(|e| e.to_string())?;
    // Drop order matters: stop the worker before removing its files.
    let mut child =
        tokio::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
            .arg("--metal-export")
            .arg(&config_file)
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(error_file)
            .spawn()
            .map_err(|e| format!("启动 Metal 导出失败: {e}"))?;
    let mut lines = BufReader::new(child.stdout.take().ok_or("缺少导出进度流")?).lines();
    let mut completed = None;
    while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
        let value: serde_json::Value =
            serde_json::from_str(&line).map_err(|e| format!("Metal 导出进度无效: {e}"))?;
        if value["ok"] == true {
            completed = Some(value);
        } else if let (Some(reporter), Some(seconds)) = (reporter, value["time_seconds"].as_f64()) {
            reporter
                .progress(StageProgress {
                    stage: "压制弹幕".into(),
                    processed: seconds,
                    total: Some(spec.duration),
                    speed: value["elapsed_seconds"]
                        .as_f64()
                        .filter(|elapsed| *elapsed > 0.0)
                        .map(|elapsed| seconds / elapsed),
                })
                .await;
        }
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    if !status.success() || completed.is_none() {
        let log = std::fs::read_to_string(&error_path).unwrap_or_default();
        let tail = log
            .lines()
            .rev()
            .take(20)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("Metal 导出失败 ({status}): {tail}"));
    }
    if !pending_output.metadata().is_ok_and(|m| m.len() > 0) {
        return Err("Metal 导出文件为空".into());
    }
    log::info!("Metal export completed: {}", completed.unwrap());
    tokio::fs::rename(&pending_output, &output)
        .await
        .map_err(|e| e.to_string())?;
    Ok(output)
}
