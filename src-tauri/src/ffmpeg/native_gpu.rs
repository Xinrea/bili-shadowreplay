//! Shared job, progress and cancellation adapter for native GPU export workers.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use super::{encoding::AvailableVideoEncoder, native_overlay::OverlaySpec};
use crate::progress::progress_reporter::{ProgressReporterTrait, StageProgress};

pub(super) enum VideoSource<'a> {
    File(&'a Path),
    /// FFmpeg writes uncompressed, normalized NV12 in a NUT stream to stdout.
    Stream(tokio::process::Command),
}

fn error_tail(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run the native pipeline in a child so task cancellation also stops FFmpeg
/// and releases GPU resources without blocking the application runtime.
pub(super) async fn encode_video(
    reporter: Option<&impl ProgressReporterTrait>,
    input: VideoSource<'_>,
    spec: OverlaySpec,
    output: &Path,
    settings: &crate::project::RenderSettings,
    encoder: &AvailableVideoEncoder,
) -> Result<PathBuf, String> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    spec.layout()?;
    let output = std::path::absolute(output)
        .map_err(|e| e.to_string())?
        .with_extension("mp4");
    let directory = tempfile::Builder::new()
        .prefix(".bili-gpu-")
        .tempdir_in(output.parent().ok_or("输出路径缺少目录")?)
        .map_err(|e| e.to_string())?;
    let pending_output = directory.path().join("output.mp4");
    let mut documents = Vec::new();
    for (index, document) in spec.input.documents.iter().enumerate() {
        let path = directory.path().join(format!("source-{index}.ass"));
        std::fs::write(&path, &document.text).map_err(|e| e.to_string())?;
        let kind = match document.kind {
            ::native_overlay::SourceKind::Subtitle => "subtitle",
            ::native_overlay::SourceKind::Danmaku => "danmaku",
        };
        documents.push(serde_json::json!({"path":path,"kind":kind}));
    }
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
    let streamed = matches!(&input, VideoSource::Stream(_));
    let source = match &input {
        VideoSource::File(path) => std::fs::canonicalize(path).map_err(|e| e.to_string())?,
        VideoSource::Stream(_) => PathBuf::from("pipe:0"),
    };
    let config = serde_json::json!({
        "source": source, "prepared": streamed,
        "output": pending_output, "documents": documents, "overlays": images,
        "fps": spec.fps, "start": spec.start,
        "encoder": encoder.encoder,
        "bitrate_bps": u64::from(settings.bitrate_kbps) * 1000,
        "options": settings.encoding.options_for(encoder, settings.priority),
    });
    let config_file = directory.path().join("job.json");
    std::fs::write(&config_file, config.to_string()).map_err(|e| e.to_string())?;
    let error_path = directory.path().join("worker.log");
    let error_file = std::fs::File::create(&error_path).map_err(|e| e.to_string())?;
    // Drop order matters: stop the worker before removing its files.
    let mut child =
        tokio::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
            .arg("--gpu-export")
            .arg(&config_file)
            .kill_on_drop(true)
            .stdin(if streamed {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(error_file)
            .spawn()
            .map_err(|e| format!("启动 GPU 导出失败: {e}"))?;
    let source_error = directory.path().join("source.log");
    let mut producer = match input {
        VideoSource::Stream(mut command) => {
            command
                .args(["-loglevel", "error", "-nostats"])
                .kill_on_drop(true)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(std::fs::File::create(&source_error).map_err(|e| e.to_string())?);
            log::info!("GPU export video source: {command:?}");
            Some(
                command
                    .spawn()
                    .map_err(|e| format!("启动切片视频处理失败: {e}"))?,
            )
        }
        VideoSource::File(_) => None,
    };
    let feed = async {
        if let Some(producer) = producer.as_mut() {
            let mut source = producer.stdout.take().ok_or("缺少切片视频流")?;
            let mut target = child.stdin.take().ok_or("缺少 GPU 视频输入")?;
            tokio::io::copy(&mut source, &mut target)
                .await
                .map_err(|e| format!("传输切片视频失败: {e}\n{}", error_tail(&error_path)))?;
            drop(target);
            let status = producer.wait().await.map_err(|e| e.to_string())?;
            if !status.success() {
                return Err(format!(
                    "切片视频处理失败 ({status}): {}",
                    error_tail(&source_error)
                ));
            }
        }
        Ok::<_, String>(())
    };
    let mut lines = BufReader::new(child.stdout.take().ok_or("缺少导出进度流")?).lines();
    let progress = async {
        let mut completed = None;
        while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
            let value: serde_json::Value =
                serde_json::from_str(&line).map_err(|e| format!("GPU 导出进度无效: {e}"))?;
            if value["ok"] == true {
                completed = Some(value);
            } else if let (Some(reporter), Some(seconds)) =
                (reporter, value["time_seconds"].as_f64())
            {
                reporter
                    .progress(StageProgress {
                        stage: if streamed {
                            "导出切片"
                        } else {
                            "压制弹幕"
                        }
                        .into(),
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
        Ok::<_, String>(completed)
    };
    let (_, completed) = tokio::try_join!(feed, progress)?;
    let status = child.wait().await.map_err(|e| e.to_string())?;
    if !status.success() || completed.is_none() {
        return Err(format!(
            "GPU 导出失败 ({status}): {}",
            error_tail(&error_path)
        ));
    }
    if streamed
        && completed
            .as_ref()
            .and_then(|value| value["frames"].as_u64())
            != Some((spec.duration * spec.fps).round() as u64)
    {
        return Err("GPU 切片分段输出帧数与时间轴不一致".into());
    }
    if !pending_output.metadata().is_ok_and(|m| m.len() > 0) {
        return Err("GPU 导出文件为空".into());
    }
    log::info!("GPU export completed: {}", completed.unwrap());
    tokio::fs::rename(&pending_output, &output)
        .await
        .map_err(|e| e.to_string())?;
    Ok(output)
}
