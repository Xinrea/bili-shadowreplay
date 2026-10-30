use std::{num::NonZeroU32, path::PathBuf, sync::OnceLock};

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
/// Maximum distance between keyframes, in frames (not seconds). Scene changes
/// and new render segments may insert earlier keyframes; B-frame policy is unchanged.
const OUTPUT_GOP_ARGS: [&str; 2] = ["-g", "120"];

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

pub(super) fn is_vulkan_encoder(encoder: &str) -> bool {
    matches!(encoder, "h264_vulkan" | "hevc_vulkan")
}

/// Part of segment identity: CPU RGB composition and GPU NV12 composition
/// have distinct colour-conversion and frame ownership policies.
pub(super) fn render_backend_identity(encoder: &str) -> &'static str {
    if is_vulkan_encoder(encoder) {
        "vulkan-nv12-overlay-v1"
    } else {
        "cpu-bgra-overlay-v1"
    }
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
    } else if TARGET_ENCODERS.contains(&encoder) || is_vulkan_encoder(encoder) {
        "format=nv12"
    } else {
        "format=yuv420p"
    }
}

/// Configure the explicit VAAPI device for optional CLI test oracles.
#[cfg(test)]
fn apply_vaapi_device(command: &mut tokio::process::Command) {
    if let Some(device_path) = vaapi_device_path() {
        command.args(["-vaapi_device", device_path.to_string_lossy().as_ref()]);
    } else {
        log::warn!("VAAPI encoder selected but no /dev/dri/renderD* device was found");
    }
}

/// Apply the shared profile to an optional CLI test oracle.
#[cfg(test)]
pub(super) fn apply_x264_quality_args(command: &mut tokio::process::Command, encoder: &str) {
    command.args(quality_args_with_bitrate(encoder, None));
}

fn with_output_gop(mut args: Vec<String>) -> Vec<String> {
    args.extend(OUTPUT_GOP_ARGS.map(str::to_owned));
    args
}

/// The encoder's quality arguments, targeting `bitrate` kbps instead of its
/// default quality when one is given. A constant-quality setting (`-crf`,
/// `-cq`) would override the target, so it is dropped; peaks and the buffer
/// keep the defaults' proportions of 1.25× and 2× the target.
pub fn quality_args_with_bitrate(encoder: &str, bitrate: Option<NonZeroU32>) -> Vec<String> {
    let defaults = quality_args_for_encoder(encoder);
    let Some(kbps) = bitrate.map(|bitrate| u64::from(bitrate.get())) else {
        return with_output_gop(defaults.iter().map(ToString::to_string).collect());
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
    with_output_gop(args)
}

/// 返回指定 H.264 编码器应使用的质量参数。
pub fn quality_args_for_encoder(encoder: &str) -> &'static [&'static str] {
    match encoder {
        "h264_vulkan" | "hevc_vulkan" => &["-rc_mode", "vbr", "-async_depth", "3", "-b:v", "8000k"],
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
/// separate: callers without settings retain their quality/rate-control policy.
pub fn project_quality_args(
    encoder: &str,
    settings: &crate::project::RenderSettings,
) -> Vec<String> {
    use crate::project::EncodingPriority;
    let quality = settings.priority == EncodingPriority::Quality;
    let options: &[&str] = match encoder {
        "h264_vulkan" | "hevc_vulkan" => &["-rc_mode", "vbr", "-async_depth", "3"],
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
    with_output_gop(args)
}

/// Choose and probe an encoder for project/overlay exports, whose native plans
/// can bind the Vulkan frame processor. Ordinary conversion callers use
/// `material_encoder` and retain their existing composition path.
pub async fn get_render_encoder(settings: &crate::project::RenderSettings) -> &'static str {
    use crate::project::{EncodingPriority, VideoCodec};
    static CACHES: [tokio::sync::OnceCell<&'static str>; 4] =
        [const { tokio::sync::OnceCell::const_new() }; 4];
    let hevc = settings.codec == VideoCodec::Hevc;
    let index = usize::from(hevc) * 2 + usize::from(settings.priority == EncodingPriority::Quality);
    CACHES[index]
        .get_or_init(|| async {
            #[cfg(all(feature = "vulkan-export", not(target_os = "macos")))]
            {
                if settings.priority == EncodingPriority::Speed {
                    let candidate = if hevc { "hevc_vulkan" } else { "h264_vulkan" };
                    if super::native_gpu::probe_encoder(candidate).await {
                        return candidate;
                    }
                } else {
                    log::info!(
                        "Quality-priority export keeps the CPU compositor and its existing encoder profile"
                    );
                }
            }
            get_cpu_render_encoder(settings).await
        })
        .await
}

/// CPU composition with the existing hardware/software encoder profiles. This
/// is selected before starting a plan when its source is outside the GPU's SDR
/// colour contract, or when GPU initialization is unavailable.
pub(super) async fn get_cpu_render_encoder(
    settings: &crate::project::RenderSettings,
) -> &'static str {
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
        Some(settings) => get_cpu_render_encoder(settings).await,
        None => get_x264_encoder().await,
    }
}

pub(super) async fn material_overlay_encoder(
    settings: Option<&crate::project::RenderSettings>,
) -> &'static str {
    match settings {
        Some(settings) => get_render_encoder(settings).await,
        None => get_x264_encoder().await,
    }
}

/// Native encoder settings from the shared profile and cache-identity policy.
/// The canonical profile strings remain stable for existing segment caches.
pub fn native_video_options(
    encoder: &str,
    filter: Option<&str>,
    bitrate: Option<NonZeroU32>,
    settings: Option<&crate::project::RenderSettings>,
) -> Result<ffmpeg_utils::VideoEncodeOptions, String> {
    if is_vulkan_encoder(encoder)
        && settings
            .is_some_and(|settings| settings.priority == crate::project::EncodingPriority::Quality)
    {
        return Err("Vulkan export currently supports speed priority; use the existing quality backend for quality priority".into());
    }
    let args = match settings {
        Some(settings) => project_quality_args(encoder, settings),
        None => quality_args_with_bitrate(encoder, bitrate),
    };
    let mut options = ffmpeg_utils::VideoEncodeOptions {
        encoder: encoder.to_owned(),
        filter: filter.map(str::to_owned),
        frame_processor: is_vulkan_encoder(encoder),
        ..Default::default()
    };
    for pair in args.as_chunks::<2>().0 {
        let (flag, value) = (pair[0].as_str(), pair[1].as_str());
        match flag {
            "-b:v" => options.bit_rate = Some(native_bit_rate(value)?),
            "-maxrate" => options.max_rate = Some(native_bit_rate(value)?),
            "-bufsize" => options.buffer_size = Some(native_bit_rate(value)?),
            "-g" => {
                options.gop = Some(
                    value
                        .parse()
                        .map_err(|error| format!("Invalid encoder GOP: {error}"))?,
                )
            }
            "-tag:v" => options.tag = Some(value.to_owned()),
            "-preset" | "-crf" | "-cq" | "-rc" | "-rc_mode" | "-quality" | "-prio_speed"
            | "-rate_control" | "-compression_level" | "-async_depth" => {
                options
                    .options
                    .insert(flag[1..].to_owned(), value.to_owned());
            }
            _ => return Err(format!("Unsupported native encoder quality option: {flag}")),
        }
    }
    if is_vaapi_encoder(encoder) {
        let device = vaapi_device_path()
            .ok_or("VAAPI encoder selected but no /dev/dri/renderD* device was found")?;
        options.hardware_device = Some(ffmpeg_utils::HardwareDevice {
            kind: "vaapi".into(),
            device: Some(device.to_string_lossy().into_owned()),
        });
        options.filter = Some(build_vaapi_filter(filter.unwrap_or_default()));
    }
    Ok(options)
}

fn native_bit_rate(value: &str) -> Result<u64, String> {
    let (digits, multiplier) = value
        .strip_suffix('k')
        .map_or((value, 1), |digits| (digits, 1000));
    digits
        .parse::<u64>()
        .ok()
        .and_then(|value| value.checked_mul(multiplier))
        .ok_or_else(|| format!("Invalid native encoder bit rate: {value}"))
}

/// Configure an encoder/filter for optional CLI test oracles.
#[cfg(test)]
pub(super) fn apply_x264_encoder_args(
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

/// Hardware H.264/HEVC encoder names registered by the linked FFmpeg SDK.
pub async fn list_supported_hwaccels() -> Result<Vec<String>, String> {
    let hwaccels: Vec<_> = ffmpeg_utils::available_video_encoders()
        .into_iter()
        .filter(|name| TARGET_ENCODERS.iter().any(|candidate| candidate == name))
        .collect();

    if hwaccels.is_empty() {
        log::info!("No H.264/HEVC hardware encoders registered in the FFmpeg SDK");
    } else {
        log::info!(
            "FFmpeg SDK registered H.264/HEVC hardware encoders: {}",
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
    let options = match native_video_options(encoder, None, None, settings) {
        Ok(options) => options,
        Err(error) => {
            log::warn!("Native video encoder probe failed: encoder={encoder}, error={error}");
            return false;
        }
    };
    match ffmpeg_utils::probe_video_encoder(options).await {
        Ok(()) => true,
        Err(error) => {
            log::warn!("Native video encoder probe failed: encoder={encoder}, error={error}");
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
        quality_args_with_bitrate(&encoder, None).join(" ")
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
    fn vulkan_speed_profile_preserves_export_limits_and_rejects_quality_downgrade() {
        let mut settings = crate::project::RenderSettings::default();
        let options =
            super::native_video_options("hevc_vulkan", None, None, Some(&settings)).unwrap();
        assert!(options.frame_processor);
        assert_eq!(options.bit_rate, Some(10_000_000));
        assert_eq!(options.gop, Some(120));
        assert_eq!(
            options.options.get("async_depth").map(String::as_str),
            Some("3")
        );
        assert_eq!(options.tag.as_deref(), Some("hvc1"));
        assert_eq!(super::output_format_filter("hevc_vulkan"), "format=nv12");
        settings.priority = crate::project::EncodingPriority::Quality;
        assert!(super::native_video_options("hevc_vulkan", None, None, Some(&settings)).is_err());
        for encoder in ["h264_vulkan", "hevc_vulkan"] {
            let legacy = super::native_video_options(encoder, None, None, None).unwrap();
            assert_eq!(
                legacy.options.get("async_depth").map(String::as_str),
                Some("3")
            );
        }
    }

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
                    .iter()
                    .copied()
                    .chain(["-g", "120"])
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn bitrate_replaces_constant_quality_for_libx264() {
        assert_eq!(
            quality_args_with_bitrate("libx264", NonZeroU32::new(12000)),
            [
                "-preset", "medium", "-b:v", "12000k", "-maxrate", "15000k", "-bufsize", "24000k",
                "-g", "120"
            ]
        );
    }

    #[test]
    fn native_profiles_preserve_quality_units_and_explicit_hevc_settings() {
        let legacy = super::native_video_options("libx264", None, None, None).unwrap();
        assert_eq!(legacy.options.get("crf").map(String::as_str), Some("20"));
        assert_eq!(
            legacy.options.get("preset").map(String::as_str),
            Some("medium")
        );
        assert_eq!(legacy.gop, Some(120));
        assert!(legacy.bit_rate.is_none());
        let target =
            super::native_video_options("libx264", None, NonZeroU32::new(12000), None).unwrap();
        assert_eq!(
            (target.bit_rate, target.max_rate, target.buffer_size),
            (Some(12_000_000), Some(15_000_000), Some(24_000_000))
        );
        assert!(!target.options.contains_key("crf"));
        let settings = crate::project::RenderSettings {
            codec: crate::project::VideoCodec::Hevc,
            priority: crate::project::EncodingPriority::Quality,
            bitrate_kbps: 6000,
        };
        let explicit = super::native_video_options(
            "libx265",
            Some(super::H264_SCALE_PAD_FILTER),
            NonZeroU32::new(12000),
            Some(&settings),
        )
        .unwrap();
        assert_eq!(explicit.encoder, "libx265");
        assert_eq!(explicit.bit_rate, Some(6_000_000));
        assert_eq!((explicit.max_rate, explicit.buffer_size), (None, None));
        assert_eq!(explicit.tag.as_deref(), Some("hvc1"));
        assert_eq!(
            explicit.options.get("preset").map(String::as_str),
            Some("slow")
        );
        assert_eq!(
            explicit.filter.as_deref(),
            Some(super::H264_SCALE_PAD_FILTER)
        );
        assert_eq!(explicit.gop, Some(120));
    }

    #[test]
    fn bitrate_replaces_hardware_targets() {
        assert_eq!(
            quality_args_with_bitrate("h264_nvenc", NonZeroU32::new(4000)),
            [
                "-preset", "p5", "-rc", "vbr", "-b:v", "4000k", "-maxrate", "5000k", "-bufsize",
                "8000k", "-g", "120"
            ]
        );
        assert_eq!(
            quality_args_with_bitrate("h264_v4l2m2m", NonZeroU32::new(4000)),
            ["-b:v", "4000k", "-g", "120"]
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
                assert_eq!(args.iter().filter(|arg| *arg == "-g").count(), 1);
                assert!(args.windows(2).any(|pair| pair == ["-g", "120"]));
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
    fn legacy_encoding_uses_the_same_gop_with_or_without_manual_bitrate() {
        for encoder in ALL_ENCODERS {
            for bitrate in [None, NonZeroU32::new(12000)] {
                let args = quality_args_with_bitrate(encoder, bitrate);
                assert_eq!(args.iter().filter(|arg| *arg == "-g").count(), 1);
                assert!(args.windows(2).any(|pair| pair == ["-g", "120"]));
                assert!(!args
                    .iter()
                    .any(|arg| ["-sc_threshold", "-keyint_min", "-bf"].contains(&arg.as_str())));
            }
        }
    }

    async fn assert_encoded_keyframe_gaps(path: &std::path::Path, context: &str) {
        let probed = ffmpeg_utils::ffprobe_command()
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "frame=key_frame",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .await
            .unwrap();
        assert!(
            probed.status.success(),
            "{context}: {}",
            String::from_utf8_lossy(&probed.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&probed.stdout).unwrap();
        let frames = value["frames"].as_array().unwrap();
        assert_eq!(frames.len(), 301, "{context}");
        let keyframes: Vec<_> = frames
            .iter()
            .enumerate()
            .filter_map(|(index, frame)| (frame["key_frame"] == 1).then_some(index))
            .collect();
        assert_eq!(keyframes.first(), Some(&0), "{context}");
        assert!(
            keyframes.windows(2).all(|pair| pair[1] - pair[0] <= 120),
            "{context}: {keyframes:?}"
        );
        assert!(
            frames.len() - keyframes.last().unwrap() <= 120,
            "{context}: {keyframes:?}"
        );
        assert!(
            keyframes.windows(2).any(|pair| pair[1] - pair[0] == 120),
            "{context}: {keyframes:?}"
        );
        eprintln!("GOP 120 verified: {context}, keyframes={keyframes:?}");
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg/ffprobe with libx264 and libx265; also tests available NVENC/VideoToolbox"]
    async fn encoded_keyframe_gaps_respect_the_120_frame_gop() {
        use crate::project::{EncodingPriority, RenderSettings, VideoCodec};
        use ffmpeg_utils::{
            ColorSource, FilterGraphPlan, GraphInput, GraphOutput, MediaInput, MediaKind,
            MediaOutput, MediaPlan, StreamMode, StreamRoute, StreamSelector, StreamSource,
        };
        let directory = tempfile::tempdir().unwrap();
        for encoder in [
            "libx264",
            "libx265",
            "h264_videotoolbox",
            "hevc_videotoolbox",
            "h264_nvenc",
            "hevc_nvenc",
        ] {
            for priority in [EncodingPriority::Speed, EncodingPriority::Quality] {
                let settings = RenderSettings {
                    codec: if encoder == "libx265" || encoder.starts_with("hevc_") {
                        VideoCodec::Hevc
                    } else {
                        VideoCodec::H264
                    },
                    priority,
                    ..RenderSettings::default()
                };
                if !super::test_encoder_with_settings(encoder, Some(&settings)).await {
                    assert!(
                        !encoder.starts_with("libx"),
                        "Required software encoder unavailable: {encoder}"
                    );
                    eprintln!("Skipping unavailable hardware encoder: {encoder} {priority:?}");
                    continue;
                }
                let path = directory.path().join("gop-cli.mp4");
                let encoded = ffmpeg_utils::ffmpeg_command()
                    .args([
                        "-v",
                        "error",
                        "-f",
                        "lavfi",
                        "-i",
                        // Match the native probe's supported 320x240 geometry.
                        // Some NVENC devices reject the old 128x96 fixture.
                        "color=c=0x204060:s=320x240:r=60",
                    ])
                    .args(["-vf", super::output_format_filter(encoder), "-c:v", encoder])
                    .args(super::project_quality_args(encoder, &settings))
                    .args(["-frames:v", "301", "-an", "-y"])
                    .arg(&path)
                    .output()
                    .await
                    .unwrap();
                assert!(
                    encoded.status.success(),
                    "{encoder}: {}",
                    String::from_utf8_lossy(&encoded.stderr)
                );
                assert_encoded_keyframe_gaps(&path, &format!("{encoder} {priority:?} CLI")).await;

                let native_path = directory.path().join("gop-native.mp4");
                let mut input = MediaInput::generated_video(ColorSource {
                    color: "0x204060".into(),
                    width: 320,
                    height: 240,
                    frame_rate_num: 60,
                    frame_rate_den: 1,
                });
                input.duration = Some(301.0 / 60.0);
                let options = super::native_video_options(
                    encoder,
                    Some(super::output_format_filter(encoder)),
                    None,
                    Some(&settings),
                )
                .unwrap();
                let plan = MediaPlan {
                    inputs: vec![input],
                    outputs: vec![MediaOutput::file(&native_path)],
                    routes: vec![StreamRoute {
                        source: StreamSource::Filter { label: "v".into() },
                        output: 0,
                        required: true,
                        mode: StreamMode::Video(options),
                    }],
                    filter_graph: Some(FilterGraphPlan {
                        spec: "[0:v]null[v]".into(),
                        inputs: vec![GraphInput {
                            input: 0,
                            selector: StreamSelector::FirstVideo,
                            label: "0:v".into(),
                            required: true,
                        }],
                        outputs: vec![GraphOutput {
                            label: "v".into(),
                            kind: MediaKind::Video,
                        }],
                    }),
                    ..Default::default()
                };
                let result = ffmpeg_utils::execute_media_plan(plan, None)
                    .await
                    .unwrap_or_else(|error| panic!("{encoder} {priority:?} native: {error}"));
                assert_eq!(result.video_frames, 301, "{encoder} {priority:?} native");
                assert_encoded_keyframe_gaps(
                    &native_path,
                    &format!("{encoder} {priority:?} native"),
                )
                .await;
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
