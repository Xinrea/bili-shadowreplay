use std::{collections::HashSet, process::Stdio, sync::OnceLock};

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

// 缓存选中的编码器，避免重复检查
static ENCODER_CACHE: OnceLock<String> = OnceLock::new();

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

/// 测试指定的编码器是否在当前硬件上真正可用
///
/// 通过尝试对测试流进行编码来验证编码器可用性
async fn test_encoder_availability(encoder: &str) -> bool {
    let mut command = tokio::process::Command::new(ffmpeg_path());

    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);

    // 使用合成输入源 (testsrc2) 测试编码器
    // -t 0.1 只编码0.1秒，-frames:v 3 只编码3帧，快速测试
    // -f null 丢弃输出，不需要实际文件
    let child = command
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg("testsrc2=duration=0.1:size=320x240:rate=1")
        .arg("-c:v")
        .arg(encoder)
        .arg("-frames:v")
        .arg("3")
        .arg("-f")
        .arg("null")
        .arg("-")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    match child {
        Ok(process) => {
            let output = process.wait_with_output().await;
            match output {
                Ok(output) => {
                    // 如果退出码为0，说明编码器可用
                    if output.status.success() {
                        log::debug!("Encoder {encoder} is available");
                        true
                    } else {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        log::debug!("Encoder {encoder} failed: {stderr}");
                        false
                    }
                }
                Err(err) => {
                    log::debug!("Encoder {encoder} test error: {err}");
                    false
                }
            }
        }
        Err(err) => {
            log::debug!("Failed to spawn ffmpeg process to test {encoder}: {err}");
            false
        }
    }
}

/// Get the preferred hardware encoder for x264
///
/// Returns the preferred hardware encoder for x264, or "libx264" if no hardware acceleration is available.
/// This function not only checks if the encoder is compiled into ffmpeg, but also verifies it's actually
/// usable on the current hardware.
///
/// The result is cached to avoid repeated checks during the program's lifetime.
pub async fn get_x264_encoder() -> &'static str {
    // 先检查缓存，如果已存在直接返回
    if let Some(encoder) = ENCODER_CACHE.get() {
        return encoder.as_str();
    }

    // 执行硬件编码器检测和验证
    let encoder = match list_supported_hwaccels().await {
        Ok(hwaccels) => {
            // 按优先级顺序测试每个硬件编码器
            const PRIORITY: [&str; 7] = [
                "h264_nvenc",
                "h264_videotoolbox",
                "h264_qsv",
                "h264_amf",
                "h264_mf",
                "h264_vaapi",
                "h264_v4l2m2m",
            ];

            let mut selected = None;
            for &candidate in &PRIORITY {
                // 检查编码器是否在支持列表中
                if hwaccels
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(candidate))
                {
                    // 测试编码器在实际硬件上是否可用
                    if test_encoder_availability(candidate).await {
                        log::info!("Found available hardware encoder: {candidate}");
                        selected = Some(candidate.to_string());
                        break;
                    } else {
                        log::debug!("Hardware encoder {candidate} is compiled in but not usable");
                    }
                }
            }

            selected.unwrap_or_else(|| {
                log::info!("No usable hardware encoder found, falling back to libx264");
                "libx264".to_string()
            })
        }
        Err(err) => {
            log::warn!("Failed to query hardware encoders: {err}");
            "libx264".to_string()
        }
    };

    log::info!("Selected x264 encoder: {}", encoder);

    // 存入缓存，如果设置成功则从缓存返回，否则返回刚才得到的值
    // 注意：set() 可能被其他线程抢先，但每个线程都会得到相同的 encoder 值
    match ENCODER_CACHE.set(encoder.clone()) {
        Ok(_) => ENCODER_CACHE.get().unwrap().as_str(),
        Err(_) => {
            // 其他线程已经设置了，返回缓存的值
            ENCODER_CACHE.get().unwrap().as_str()
        }
    }
}
