use std::{collections::HashSet, num::NonZeroU32, path::PathBuf, process::Stdio, sync::OnceLock};

use ffmpeg_utils::ffmpeg_command;
use tokio::io::AsyncReadExt;

const TARGET_ENCODERS: [&str; 14] = [
    "h264_nvenc",
    "h264_videotoolbox",
    "h264_qsv",
    "h264_amf",
    "h264_mf",
    "h264_vaapi",
    "h264_v4l2m2m",
    "hevc_nvenc",
    "hevc_videotoolbox",
    "hevc_qsv",
    "hevc_amf",
    "hevc_mf",
    "hevc_vaapi",
    "hevc_v4l2m2m",
];

pub const H264_SCALE_PAD_FILTER: &str =
    "scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2";

const VAAPI_ENCODER: &str = "h264_vaapi";
const NVENC_ENCODER: &str = "h264_nvenc";
const VIDEOTOOLBOX_ENCODER: &str = "h264_videotoolbox";
const QSV_ENCODER: &str = "h264_qsv";
const AMF_ENCODER: &str = "h264_amf";
const MF_ENCODER: &str = "h264_mf";
const V4L2M2M_ENCODER: &str = "h264_v4l2m2m";
const VAAPI_DEVICE_DIR: &str = "/dev/dri";
const VAAPI_RENDER_PREFIX: &str = "renderD";
const VAAPI_FILTER_SUFFIX: &str = "format=nv12,hwupload";

// 缓存选中的编码器，避免重复检查
static ENCODER_CACHE: OnceLock<String> = OnceLock::new();

/// 查找 Linux VAAPI render 设备。
fn vaapi_device_path() -> Option<PathBuf> {
    let entries = std::fs::read_dir(VAAPI_DEVICE_DIR).ok()?;
    let mut devices = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(VAAPI_RENDER_PREFIX))
        })
        .collect::<Vec<_>>();

    devices.sort();
    devices.into_iter().next()
}

/// 为 VAAPI 滤镜链追加硬件上传步骤。
fn build_vaapi_filter(filter: &str) -> String {
    if filter.trim().is_empty() {
        VAAPI_FILTER_SUFFIX.to_string()
    } else {
        format!("{filter},{VAAPI_FILTER_SUFFIX}")
    }
}

/// 判断编码器是否为 VAAPI。
pub fn is_vaapi_encoder(encoder: &str) -> bool {
    matches!(encoder, "h264_vaapi" | "hevc_vaapi")
}

/// 返回 VAAPI 硬件上传滤镜片段。
pub fn vaapi_filter_suffix() -> &'static str {
    VAAPI_FILTER_SUFFIX
}

/// Convert the finished composition once for the selected encoder. Hardware
/// encoders consume NV12; VAAPI also needs an upload. libx265 requires planar
/// YUV, so retain yuv420p for software encoding rather than imposing NV12.
pub fn output_format_filter(encoder: &str) -> &'static str {
    if is_vaapi_encoder(encoder) {
        vaapi_filter_suffix()
    } else if TARGET_ENCODERS.contains(&encoder) {
        "format=nv12"
    } else {
        "format=yuv420p"
    }
}

/// 为 FFmpeg 命令追加 VAAPI 设备参数。
fn apply_vaapi_device(command: &mut tokio::process::Command) {
    if let Some(device_path) = vaapi_device_path() {
        command.args(["-vaapi_device", device_path.to_string_lossy().as_ref()]);
    } else {
        log::warn!("VAAPI encoder selected but no /dev/dri/renderD* device was found");
    }
}

/// 只追加视频编码器参数，不追加视频滤镜。
pub fn apply_x264_encoder_only(command: &mut tokio::process::Command, encoder: &str) {
    if is_vaapi_encoder(encoder) {
        apply_vaapi_device(command);
    }
    command.args(["-c:v", encoder]);
}

/// 按编码器类型追加质量参数。
pub fn apply_x264_quality_args(command: &mut tokio::process::Command, encoder: &str) {
    apply_x264_quality_args_with_bitrate(command, encoder, None);
}

/// 按编码器类型追加质量参数；指定 `bitrate`（kbps）时改用该目标码率。
pub fn apply_x264_quality_args_with_bitrate(
    command: &mut tokio::process::Command,
    encoder: &str,
    bitrate: Option<NonZeroU32>,
) {
    let quality_args = quality_args_with_bitrate(encoder, bitrate);
    log::info!(
        "Applying H.264 quality parameters: encoder={encoder}, args={}",
        quality_args.join(" ")
    );
    command.args(quality_args);
}

/// The encoder's quality arguments, targeting `bitrate` kbps instead of its
/// default quality when one is given. A constant-quality setting (`-crf`,
/// `-cq`) would override the target, so it is dropped; peaks and the buffer
/// keep the defaults' proportions of 1.25× and 2× the target.
pub fn quality_args_with_bitrate(encoder: &str, bitrate: Option<NonZeroU32>) -> Vec<String> {
    let defaults = quality_args_for_encoder(encoder);
    let Some(kbps) = bitrate.map(|bitrate| u64::from(bitrate.get())) else {
        return defaults.iter().map(ToString::to_string).collect();
    };
    let rate = |flag: &str| match flag {
        "-b:v" => Some(kbps),
        "-maxrate" => Some(kbps * 5 / 4),
        "-bufsize" => Some(kbps * 2),
        _ => None,
    };
    let mut args = Vec::with_capacity(defaults.len() + 6);
    let mut has_target = false;
    for pair in defaults.chunks(2) {
        let (flag, value) = (pair[0], pair.get(1).copied().unwrap_or_default());
        if matches!(flag, "-crf" | "-cq") {
            continue;
        }
        has_target |= flag == "-b:v";
        args.push(flag.to_string());
        args.push(rate(flag).map_or_else(|| value.to_string(), |rate| format!("{rate}k")));
    }
    // Constant-quality encoders such as libx264 have no target to replace.
    if !has_target {
        for flag in ["-b:v", "-maxrate", "-bufsize"] {
            args.push(flag.to_string());
            args.push(format!("{}k", rate(flag).unwrap_or(kbps)));
        }
    }
    args
}

/// 返回指定 H.264 编码器应使用的质量参数。
pub fn quality_args_for_encoder(encoder: &str) -> &'static [&'static str] {
    match encoder {
        VAAPI_ENCODER => &["-b:v", "8000k", "-maxrate", "10000k", "-bufsize", "16000k"],
        NVENC_ENCODER => &[
            "-preset", "p5", "-rc", "vbr", "-cq", "23", "-b:v", "8000k", "-maxrate", "10000k",
            "-bufsize", "16000k",
        ],
        VIDEOTOOLBOX_ENCODER => &["-b:v", "8000k", "-maxrate", "10000k", "-bufsize", "16000k"],
        QSV_ENCODER => &[
            "-preset", "medium", "-b:v", "8000k", "-maxrate", "10000k", "-bufsize", "16000k",
        ],
        AMF_ENCODER => &[
            "-quality", "quality", "-rc", "vbr_peak", "-b:v", "8000k", "-maxrate", "10000k",
            "-bufsize", "16000k",
        ],
        MF_ENCODER => &[
            "-rate_control",
            "pc_vbr",
            "-b:v",
            "8000k",
            "-maxrate",
            "10000k",
            "-bufsize",
            "16000k",
        ],
        V4L2M2M_ENCODER => &["-b:v", "6000k"],
        _ => &["-crf", "20", "-preset", "medium"],
    }
}

/// Explicit material/export/upload settings. Keep legacy H.264 quality helpers
/// separate: callers without settings retain their historical encoding policy.
pub fn project_quality_args(
    encoder: &str,
    settings: &crate::project::RenderSettings,
) -> Vec<String> {
    use crate::project::EncodingPriority;
    let quality = settings.priority == EncodingPriority::Quality;
    let options: &[&str] = match encoder {
        "h264_nvenc" | "hevc_nvenc" => {
            &["-preset", if quality { "p7" } else { "p1" }, "-rc", "vbr"]
        }
        "h264_videotoolbox" | "hevc_videotoolbox" => {
            // FFmpeg's name for VideoToolbox's PrioritizeEncodingSpeedOverQuality.
            &["-prio_speed", if quality { "0" } else { "1" }]
        }
        "h264_qsv" | "hevc_qsv" => &["-preset", if quality { "slow" } else { "veryfast" }],
        "h264_amf" | "hevc_amf" => &[
            "-quality",
            if quality { "quality" } else { "speed" },
            "-rc",
            "vbr_peak",
        ],
        "h264_mf" | "hevc_mf" => &["-rate_control", "pc_vbr"],
        "h264_vaapi" | "hevc_vaapi" => &["-compression_level", if quality { "1" } else { "7" }],
        "h264_v4l2m2m" | "hevc_v4l2m2m" => &[],
        _ => &["-preset", if quality { "slow" } else { "veryfast" }],
    };
    let mut args: Vec<String> = options.iter().map(ToString::to_string).collect();
    args.extend(["-b:v".to_string(), format!("{}k", settings.bitrate_kbps)]);
    // Tag HEVC for MP4 playback on Apple clients. Never force B-frame counts.
    if encoder.starts_with("hevc_") || encoder == "libx265" {
        args.extend(["-tag:v".to_string(), "hvc1".to_string()]);
    }
    args
}

/// Choose and probe an encoder for each codec/priority combination. Unlike the
/// legacy selector, the probe uses exactly the options the render will use.
pub async fn get_render_encoder(settings: &crate::project::RenderSettings) -> &'static str {
    use crate::project::{EncodingPriority, VideoCodec};
    static CACHES: [tokio::sync::OnceCell<&'static str>; 4] =
        [const { tokio::sync::OnceCell::const_new() }; 4];
    let hevc = settings.codec == VideoCodec::Hevc;
    let index = usize::from(hevc) * 2 + usize::from(settings.priority == EncodingPriority::Quality);
    CACHES[index]
        .get_or_init(|| async {
            let available = list_supported_hwaccels().await.unwrap_or_default();
            for &candidate in &TARGET_ENCODERS {
                if candidate.starts_with(if hevc { "hevc_" } else { "h264_" })
                    && available.iter().any(|name| name == candidate)
                    && test_encoder_with_settings(candidate, Some(settings)).await
                {
                    return candidate;
                }
            }
            if hevc {
                "libx265"
            } else {
                "libx264"
            }
        })
        .await
}

/// Optional settings keep old material callers on the old H.264 path.
pub async fn material_encoder(settings: Option<&crate::project::RenderSettings>) -> &'static str {
    match settings {
        Some(settings) => get_render_encoder(settings).await,
        None => get_x264_encoder().await,
    }
}

pub fn apply_material_quality(
    command: &mut tokio::process::Command,
    encoder: &str,
    bitrate: Option<NonZeroU32>,
    settings: Option<&crate::project::RenderSettings>,
) {
    match settings {
        Some(settings) => {
            command.args(project_quality_args(encoder, settings));
        }
        None => apply_x264_quality_args_with_bitrate(command, encoder, bitrate),
    }
}

/// 追加视频编码器参数，并在 VAAPI 下补齐硬件上传滤镜。
pub fn apply_x264_encoder_args(
    command: &mut tokio::process::Command,
    encoder: &str,
    filter: Option<&str>,
) {
    if is_vaapi_encoder(encoder) {
        apply_vaapi_device(command);

        let filter = filter
            .map(build_vaapi_filter)
            .unwrap_or_else(|| VAAPI_FILTER_SUFFIX.to_string());
        command.args(["-vf", filter.as_str()]);
        command.args(["-c:v", encoder]);
    } else {
        if let Some(filter) = filter {
            command.args(["-vf", filter]);
        }
        command.args(["-c:v", encoder]);
    }
}

/// 检测当前环境下 FFmpeg 支持的 H.264 硬件编码器。
///
/// 返回值为可直接用于 `-c:v <value>` 的编码器名称列表。
pub async fn list_supported_hwaccels() -> Result<Vec<String>, String> {
    let mut command = ffmpeg_command();

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

    if hwaccels.is_empty() {
        log::info!("No supported H.264 hardware encoders reported by FFmpeg");
    } else {
        log::info!(
            "FFmpeg reported supported H.264 hardware encoders: {}",
            hwaccels.join(", ")
        );
    }

    Ok(hwaccels)
}

/// 测试指定的编码器是否在当前硬件上真正可用
///
/// 通过尝试对测试流进行编码来验证编码器可用性
async fn test_encoder_availability(encoder: &str) -> bool {
    test_encoder_with_settings(encoder, None).await
}

async fn test_encoder_with_settings(
    encoder: &str,
    settings: Option<&crate::project::RenderSettings>,
) -> bool {
    let mut command = ffmpeg_command();

    // 使用合成输入源 (testsrc2) 测试编码器
    // -frames:v 3 只编码3帧，快速测试
    // -f null 丢弃输出，不需要实际文件
    command.arg("-hide_banner").arg("-loglevel").arg("error");

    let filter = if is_vaapi_encoder(encoder) {
        if let Some(device_path) = vaapi_device_path() {
            command.args(["-vaapi_device", device_path.to_string_lossy().as_ref()]);
            Some(vaapi_filter_suffix())
        } else {
            log::warn!(
                "H.264 encoder test skipped: encoder={encoder}, reason=no /dev/dri/renderD* device"
            );
            return false;
        }
    } else {
        None
    };

    command
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg("testsrc2=duration=1:size=320x240:rate=30");

    if let Some(filter) = filter {
        command.args(["-vf", filter]);
    }
    command.args(["-c:v", encoder]);
    let quality_args = match settings {
        Some(settings) => project_quality_args(encoder, settings),
        None => quality_args_with_bitrate(encoder, None),
    };
    log::info!(
        "Testing H.264 encoder: encoder={encoder}, quality_args={}",
        quality_args.join(" ")
    );
    command.args(&quality_args);

    let child = command
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
                        log::info!(
                            "H.264 encoder test succeeded: encoder={encoder}, quality_args={}",
                            quality_args.join(" ")
                        );
                        true
                    } else {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        log::warn!(
                            "H.264 encoder test failed: encoder={encoder}, quality_args={}, stderr={}",
                            quality_args.join(" "),
                            stderr.trim()
                        );
                        false
                    }
                }
                Err(err) => {
                    log::warn!("H.264 encoder test error: encoder={encoder}, error={err}");
                    false
                }
            }
        }
        Err(err) => {
            log::warn!("Failed to start H.264 encoder test: encoder={encoder}, error={err}");
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

    log::info!(
        "Selected H.264 encoder: encoder={}, quality_args={}",
        encoder,
        quality_args_for_encoder(&encoder).join(" ")
    );

    // 存入缓存，如果设置成功则从缓存返回，否则返回刚才得到的值
    // 注意：set() 可能被其他线程抢先，但每个线程都会得到相同的 encoder 值
    let _ = ENCODER_CACHE.set(encoder);
    // 其他线程可能抢先设置，但每个线程都会得到相同的 encoder 值
    ENCODER_CACHE.get().map(String::as_str).unwrap_or("libx264")
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::{quality_args_for_encoder, quality_args_with_bitrate};

    const ALL_ENCODERS: [&str; 8] = [
        "libx264",
        "h264_nvenc",
        "h264_videotoolbox",
        "h264_qsv",
        "h264_amf",
        "h264_mf",
        "h264_vaapi",
        "h264_v4l2m2m",
    ];

    #[test]
    fn composited_output_uses_nv12_only_for_hardware_encoders() {
        for encoder in super::TARGET_ENCODERS {
            let expected = if super::is_vaapi_encoder(encoder) {
                "format=nv12,hwupload"
            } else {
                "format=nv12"
            };
            assert_eq!(super::output_format_filter(encoder), expected, "{encoder}");
        }
        for encoder in ["libx264", "libx265"] {
            assert_eq!(super::output_format_filter(encoder), "format=yuv420p");
        }
    }

    #[test]
    fn quality_args_are_flag_value_pairs() {
        for encoder in ALL_ENCODERS {
            assert_eq!(quality_args_for_encoder(encoder).len() % 2, 0, "{encoder}");
        }
    }

    #[test]
    fn no_bitrate_keeps_the_default_quality() {
        for encoder in ALL_ENCODERS {
            assert_eq!(
                quality_args_with_bitrate(encoder, None),
                quality_args_for_encoder(encoder)
            );
        }
    }

    #[test]
    fn bitrate_replaces_constant_quality_for_libx264() {
        assert_eq!(
            quality_args_with_bitrate("libx264", NonZeroU32::new(12000)),
            ["-preset", "medium", "-b:v", "12000k", "-maxrate", "15000k", "-bufsize", "24000k"]
        );
    }

    #[test]
    fn bitrate_replaces_hardware_targets() {
        assert_eq!(
            quality_args_with_bitrate("h264_nvenc", NonZeroU32::new(4000)),
            [
                "-preset", "p5", "-rc", "vbr", "-b:v", "4000k", "-maxrate", "5000k", "-bufsize",
                "8000k"
            ]
        );
        assert_eq!(
            quality_args_with_bitrate("h264_v4l2m2m", NonZeroU32::new(4000)),
            ["-b:v", "4000k"]
        );
    }

    #[test]
    fn explicit_settings_use_bitrate_without_constant_quality_or_b_frame_overrides() {
        use crate::project::{EncodingPriority, RenderSettings};
        for encoder in super::TARGET_ENCODERS
            .into_iter()
            .chain(["libx264", "libx265"])
        {
            for priority in [EncodingPriority::Speed, EncodingPriority::Quality] {
                let args = super::project_quality_args(
                    encoder,
                    &RenderSettings {
                        priority,
                        bitrate_kbps: 12_000,
                        ..RenderSettings::default()
                    },
                );
                assert_eq!(args.len() % 2, 0);
                assert!(args.windows(2).any(|pair| pair == ["-b:v", "12000k"]));
                assert!(
                    !args.iter().any(|arg| {
                        ["-crf", "-cq", "-maxrate", "-bufsize", "-bf", "-b_ref_mode"]
                            .contains(&arg.as_str())
                    }),
                    "{encoder}: {args:?}"
                );
            }
        }
    }

    #[test]
    fn explicit_priority_mapping_matches_benchmarked_encoders_and_software() {
        use crate::project::{EncodingPriority, RenderSettings};
        for (suffix, flag, speed, quality) in [
            ("nvenc", "-preset", "p1", "p7"),
            ("videotoolbox", "-prio_speed", "1", "0"),
            ("software", "-preset", "veryfast", "slow"),
        ] {
            for codec in ["h264", "hevc"] {
                let encoder = if suffix == "software" {
                    if codec == "h264" {
                        "libx264".into()
                    } else {
                        "libx265".into()
                    }
                } else {
                    format!("{codec}_{suffix}")
                };
                for (priority, value) in [
                    (EncodingPriority::Speed, speed),
                    (EncodingPriority::Quality, quality),
                ] {
                    let args = super::project_quality_args(
                        &encoder,
                        &RenderSettings {
                            priority,
                            ..RenderSettings::default()
                        },
                    );
                    assert!(
                        args.windows(2).any(|pair| pair == [flag, value]),
                        "{args:?}"
                    );
                    if suffix == "nvenc" {
                        assert!(args.windows(2).any(|pair| pair == ["-rc", "vbr"]));
                    }
                    assert_eq!(args.iter().any(|arg| arg == "hvc1"), codec == "hevc");
                }
            }
        }
    }

    #[test]
    fn test_quality_args_for_libx264() {
        assert_eq!(
            quality_args_for_encoder("libx264"),
            &["-crf", "20", "-preset", "medium"]
        );
    }

    #[test]
    fn test_quality_args_for_nvenc() {
        let args = quality_args_for_encoder("h264_nvenc");

        assert_eq!(
            args,
            &[
                "-preset", "p5", "-rc", "vbr", "-cq", "23", "-b:v", "8000k", "-maxrate", "10000k",
                "-bufsize", "16000k"
            ]
        );
        assert!(!args.contains(&"-crf"));
    }

    #[test]
    fn test_quality_args_for_vaapi() {
        assert_eq!(
            quality_args_for_encoder("h264_vaapi"),
            &["-b:v", "8000k", "-maxrate", "10000k", "-bufsize", "16000k"]
        );
    }

    #[test]
    fn test_quality_args_for_videotoolbox() {
        assert_eq!(
            quality_args_for_encoder("h264_videotoolbox"),
            &["-b:v", "8000k", "-maxrate", "10000k", "-bufsize", "16000k"]
        );
    }

    #[test]
    fn test_quality_args_for_qsv() {
        assert_eq!(
            quality_args_for_encoder("h264_qsv"),
            &["-preset", "medium", "-b:v", "8000k", "-maxrate", "10000k", "-bufsize", "16000k"]
        );
    }

    #[test]
    fn test_quality_args_for_amf() {
        assert_eq!(
            quality_args_for_encoder("h264_amf"),
            &[
                "-quality", "quality", "-rc", "vbr_peak", "-b:v", "8000k", "-maxrate", "10000k",
                "-bufsize", "16000k"
            ]
        );
    }

    #[test]
    fn test_quality_args_for_media_foundation() {
        assert_eq!(
            quality_args_for_encoder("h264_mf"),
            &[
                "-rate_control",
                "pc_vbr",
                "-b:v",
                "8000k",
                "-maxrate",
                "10000k",
                "-bufsize",
                "16000k"
            ]
        );
    }

    #[test]
    fn test_hardware_encoders_have_bitrate_targets() {
        for encoder in [
            "h264_nvenc",
            "h264_videotoolbox",
            "h264_qsv",
            "h264_amf",
            "h264_mf",
            "h264_vaapi",
            "h264_v4l2m2m",
        ] {
            assert!(
                quality_args_for_encoder(encoder).contains(&"-b:v"),
                "{encoder} must have a target bitrate"
            );
        }
    }

    #[test]
    fn test_vbr_hardware_encoders_have_peak_constraints() {
        for encoder in [
            "h264_nvenc",
            "h264_videotoolbox",
            "h264_qsv",
            "h264_amf",
            "h264_mf",
            "h264_vaapi",
        ] {
            let args = quality_args_for_encoder(encoder);
            assert!(
                args.contains(&"-maxrate") && args.contains(&"-bufsize"),
                "{encoder} must constrain bitrate peaks"
            );
        }
    }

    #[test]
    fn test_quality_args_for_v4l2m2m() {
        assert_eq!(quality_args_for_encoder("h264_v4l2m2m"), &["-b:v", "6000k"]);
    }
}
