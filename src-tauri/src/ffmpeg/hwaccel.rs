use std::{collections::HashSet, process::Stdio};

use tokio::io::AsyncReadExt;

use super::ffmpeg_path;

const TARGET_ENCODERS: [&str; 7] = [
    "h264_nvenc",
    "h264_videotoolbox",
    "h264_qsv",
    "h264_amf",
    "h264_mf",
    "h264_vaapi",
    "h264_v4l2m2m",
];

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// 检测当前环境下 FFmpeg 支持的 H.264 硬件编码器。
///
/// 返回值为可直接用于 `-c:v <value>` 的编码器名称列表。
pub async fn list_supported_hwaccels() -> Result<Vec<String>, String> {
    let mut command = tokio::process::Command::new(ffmpeg_path());

    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child = command
        .arg("-hide_banner")
        .arg("-encoders")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法启动 ffmpeg 进程: {e}"))?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法获取 ffmpeg 标准输出".to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "无法获取 ffmpeg 标准错误输出".to_string())?;

    let (mut stdout_buf, mut stderr_buf) = (String::new(), String::new());

    let stdout_future = stdout.read_to_string(&mut stdout_buf);
    let stderr_future = stderr.read_to_string(&mut stderr_buf);

    let (stdout_res, stderr_res, status) = tokio::join!(stdout_future, stderr_future, child.wait());

    stdout_res.map_err(|e| format!("读取 ffmpeg 标准输出失败: {e}"))?;
    stderr_res.map_err(|e| format!("读取 ffmpeg 标准错误输出失败: {e}"))?;

    let status = status.map_err(|e| format!("等待 ffmpeg 进程退出失败: {e}"))?;

    if !status.success() {
        let err = if stderr_buf.trim().is_empty() {
            stdout_buf.trim().to_string()
        } else {
            stderr_buf.trim().to_string()
        };
        log::error!("ffmpeg -encoders 运行失败: {err}");
        return Err(format!("ffmpeg -encoders 运行失败: {err}"));
    }

    let mut hwaccels = Vec::new();

    for line in stdout_buf.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || !trimmed.starts_with('V') {
            continue;
        }

        let mut parts = trimmed.split_whitespace();
        let flags = parts.next().unwrap_or_default();
        if !flags.starts_with('V') {
            continue;
        }

        if let Some(name) = parts.next() {
            if TARGET_ENCODERS
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(name))
            {
                hwaccels.push(name.to_string());
            }
        }
    }

    // 去重并保持原有顺序（即第一次出现时保留）
    let mut seen = HashSet::new();
    hwaccels.retain(|value| seen.insert(value.clone()));

    Ok(hwaccels)
}

/// 依据优先级从支持列表中挑选推荐的硬件编码器。
///
/// 当前优先级顺序：`h264_nvenc` > `h264_videotoolbox` > `h264_qsv` > `h264_amf` > `h264_mf` > `h264_vaapi` > `h264_v4l2m2m`。
pub fn select_preferred_hwaccel(supported: &[String]) -> Option<&'static str> {
    const PRIORITY: [&str; 7] = [
        "h264_nvenc",
        "h264_videotoolbox",
        "h264_qsv",
        "h264_amf",
        "h264_mf",
        "h264_vaapi",
        "h264_v4l2m2m",
    ];

    PRIORITY
        .iter()
        .find(|candidate| {
            supported
                .iter()
                .any(|value| value.eq_ignore_ascii_case(candidate))
        })
        .copied()
}

/// Get the preferred hardware encoder for x264
///
/// Returns the preferred hardware encoder for x264, or "libx264" if no hardware acceleration is available.
pub async fn get_x264_encoder() -> &'static str {
    let mut encoder = "libx264";
    match list_supported_hwaccels().await {
        Ok(hwaccels) => {
            if let Some(arg) = select_preferred_hwaccel(&hwaccels) {
                encoder = arg;
            }
        }
        Err(err) => {
            log::warn!("Failed to query hardware encoders: {err}");
        }
    }

    log::info!("Selected x264 encoder: {encoder}");
    encoder
}
