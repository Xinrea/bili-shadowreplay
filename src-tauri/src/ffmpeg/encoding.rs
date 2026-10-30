//! Encoder discovery and settings shared by native GPU and FFmpeg exports.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;

pub use crate::project::VideoCodec;
use crate::project::{EncodingPriority, RenderSettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoRenderBackend {
    Cpu,
    Metal,
    Vulkan,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoEncodingSettings {
    pub render_backend: Option<VideoRenderBackend>,
    pub encoder: Option<String>,
    pub options: BTreeMap<String, String>,
}

impl VideoEncodingSettings {
    pub async fn encoder(
        &self,
        codec: VideoCodec,
        native_gpu: bool,
    ) -> Result<&'static AvailableVideoEncoder, String> {
        let available = available_encoders().await?;
        let encoder = available
            .iter()
            .find(|item| {
                item.codec == codec
                    && (native_gpu || !item.is_native())
                    && self
                        .render_backend
                        .is_none_or(|backend| item.render_backend == backend)
                    && self
                        .encoder
                        .as_ref()
                        .is_none_or(|name| item.encoder == *name)
            })
            .ok_or("所选编码器在当前导出流程中不可用，请重新选择")?;
        for (name, value) in &self.options {
            let option = encoder
                .options
                .iter()
                .find(|option| option.name == name)
                .ok_or_else(|| format!("{} 不支持参数 {name}", encoder.encoder))?;
            let valid = match option.range {
                Some([min, max]) => value.parse::<i32>().is_ok_and(|v| (min..=max).contains(&v)),
                None => option.choices.iter().any(|(v, _)| *v == value.as_str()),
            };
            if !valid {
                return Err(format!("{} 的值无效: {value}", option.label));
            }
        }
        Ok(encoder)
    }

    pub(crate) fn options_for(
        &self,
        encoder: &AvailableVideoEncoder,
        priority: EncodingPriority,
    ) -> BTreeMap<String, String> {
        let defaults = super::hwaccel::project_quality_args(
            &encoder.encoder,
            &RenderSettings {
                codec: encoder.codec,
                priority,
                ..RenderSettings::default()
            },
        );
        encoder
            .options
            .iter()
            .map(|option| {
                let default = if encoder.render_backend == VideoRenderBackend::Vulkan
                    && option.name == "quality"
                {
                    if priority == EncodingPriority::Quality {
                        option.range.unwrap()[1]
                    } else {
                        0
                    }
                    .to_string()
                } else {
                    defaults
                        .chunks(2)
                        .find(|pair| pair[0].strip_prefix('-') == Some(option.name))
                        .map_or_else(|| option.default.clone(), |pair| pair[1].clone())
                };
                (
                    option.name.to_owned(),
                    self.options.get(option.name).cloned().unwrap_or(default),
                )
            })
            .collect()
    }
}

/// The same definitions drive UI controls, validation and FFmpeg arguments.
#[derive(Debug, Clone, Serialize)]
pub struct VideoEncoderOption {
    pub name: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    pub default: String,
    pub choices: Vec<(&'static str, &'static str)>,
    pub range: Option<[i32; 2]>,
}

impl VideoEncoderOption {
    fn number(
        name: &'static str,
        label: &'static str,
        hint: &'static str,
        default: &'static str,
        min: i32,
        max: i32,
    ) -> Self {
        Self {
            name,
            label,
            hint,
            default: default.into(),
            choices: vec![],
            range: Some([min, max]),
        }
    }

    fn choice(
        name: &'static str,
        label: &'static str,
        hint: &'static str,
        default: &'static str,
        choices: &[(&'static str, &'static str)],
    ) -> Self {
        Self {
            name,
            label,
            hint,
            default: default.into(),
            choices: choices.to_vec(),
            range: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AvailableVideoEncoder {
    pub codec: VideoCodec,
    pub encoder: String,
    pub render_backend: VideoRenderBackend,
    pub options: Vec<VideoEncoderOption>,
}

impl AvailableVideoEncoder {
    pub fn is_native(&self) -> bool {
        self.render_backend != VideoRenderBackend::Cpu
    }

    fn new(
        codec: VideoCodec,
        encoder: &str,
        render_backend: VideoRenderBackend,
        quality_max: i32,
    ) -> Self {
        use VideoEncoderOption as Param;
        let videotoolbox = encoder.ends_with("_videotoolbox");
        let vulkan = encoder.ends_with("_vulkan");
        let software = matches!(encoder, "libx264" | "libx265");
        let mut options = Vec::new();
        if videotoolbox {
            options.push(Param::choice(
                "prio_speed",
                "编码策略",
                "VideoToolbox 的速度与质量取舍。",
                "1",
                &[("1", "速度优先"), ("0", "质量优先")],
            ));
        } else if vulkan {
            options.push(Param::number(
                "quality",
                "编码质量等级",
                "Vulkan Video 的速度与质量等级，范围来自当前设备。",
                "0",
                0,
                quality_max,
            ));
        } else if encoder.ends_with("_nvenc") {
            options.push(Param::choice(
                "preset",
                "编码预设",
                "P1 最快，P7 最慢；较慢的预设通常提高压缩效率。",
                "p5",
                &[
                    ("p1", "P1"),
                    ("p2", "P2"),
                    ("p3", "P3"),
                    ("p4", "P4"),
                    ("p5", "P5"),
                    ("p6", "P6"),
                    ("p7", "P7"),
                ],
            ));
            options.push(Param::number(
                "cq",
                "CQ",
                "VBR 目标质量：0 为自动，1–51 越小质量越高；仍受目标码率影响。",
                "0",
                0,
                51,
            ));
        } else if software || encoder.ends_with("_qsv") {
            let mut presets = vec![
                ("veryfast", "veryfast"),
                ("faster", "faster"),
                ("fast", "fast"),
                ("medium", "medium"),
                ("slow", "slow"),
                ("slower", "slower"),
                ("veryslow", "veryslow"),
            ];
            if software {
                presets.splice(
                    0..0,
                    [("ultrafast", "ultrafast"), ("superfast", "superfast")],
                );
            }
            options.push(Param::choice(
                "preset",
                "编码预设",
                "越快的预设通常需要更高码率才能保持相近画质。",
                "medium",
                &presets,
            ));
            if software {
                options.push(Param::number(
                    "crf",
                    "CRF",
                    "-1 使用目标码率；0–51 按质量编码，越小质量越高；最大码率为所设码率的 1.25 倍。",
                    "-1",
                    -1,
                    51,
                ));
            }
        } else if encoder.ends_with("_amf") {
            options.push(Param::choice(
                "quality",
                "编码预设",
                "AMF 的速度与质量取舍。",
                "quality",
                &[("speed", "速度"), ("balanced", "均衡"), ("quality", "质量")],
            ));
        }
        Self {
            codec,
            encoder: encoder.into(),
            render_backend,
            options,
        }
    }
}

static ENCODERS: OnceCell<Vec<AvailableVideoEncoder>> = OnceCell::const_new();

pub async fn available_encoders() -> Result<&'static Vec<AvailableVideoEncoder>, String> {
    ENCODERS
        .get_or_try_init(|| async {
            let mut result = Vec::new();
            #[cfg(any(
                all(target_os = "macos", target_arch = "aarch64"),
                all(
                    feature = "vulkan-export",
                    any(target_os = "linux", target_os = "windows")
                )
            ))]
            {
                match tokio::task::spawn_blocking(gpu_export::codecs).await {
                    Ok(Ok(codecs)) => {
                        let (render_backend, suffix) = if cfg!(target_os = "macos") {
                            (VideoRenderBackend::Metal, "videotoolbox")
                        } else {
                            (VideoRenderBackend::Vulkan, "vulkan")
                        };
                        for (i, (codec, name)) in
                            [(VideoCodec::H264, "h264"), (VideoCodec::Hevc, "hevc")]
                                .into_iter()
                                .enumerate()
                        {
                            if codecs[i].0 {
                                result.push(AvailableVideoEncoder::new(
                                    codec,
                                    &format!("{name}_{suffix}"),
                                    render_backend,
                                    codecs[i].1,
                                ));
                            }
                        }
                        if cfg!(target_os = "macos") {
                            result.reverse();
                        }
                    }
                    other => log::warn!("Native GPU encoders unavailable: {other:?}"),
                }
            }
            for (codec, candidates) in [
                (
                    VideoCodec::H264,
                    [
                        "h264_nvenc",
                        "h264_videotoolbox",
                        "h264_qsv",
                        "h264_amf",
                        "h264_mf",
                        "h264_vaapi",
                        "h264_v4l2m2m",
                        "libx264",
                    ],
                ),
                (
                    VideoCodec::Hevc,
                    [
                        "hevc_nvenc",
                        "hevc_videotoolbox",
                        "hevc_qsv",
                        "hevc_amf",
                        "hevc_mf",
                        "hevc_vaapi",
                        "hevc_v4l2m2m",
                        "libx265",
                    ],
                ),
            ] {
                for encoder in candidates {
                    if super::hwaccel::test_encoder_availability(encoder).await {
                        result.push(AvailableVideoEncoder::new(
                            codec,
                            encoder,
                            VideoRenderBackend::Cpu,
                            0,
                        ));
                    }
                }
            }
            if !result
                .iter()
                .any(|item| matches!(item.encoder.as_str(), "libx264" | "libx265"))
            {
                return Err(
                    "FFmpeg 缺少可用的 CPU 软件编码器，请安装启用 libx264 或 libx265 的版本".into(),
                );
            }
            Ok(result)
        })
        .await
}
