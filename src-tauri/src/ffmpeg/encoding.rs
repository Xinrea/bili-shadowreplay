//! Shared codec selection and encoding settings for native and FFmpeg exports.

use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    H264,
    Hevc,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoEncodingSettings {
    pub codec: VideoCodec,
    pub bitrate_mbps: u32,
    pub videotoolbox_speed_priority: bool,
}

impl Default for VideoEncodingSettings {
    fn default() -> Self {
        Self {
            codec: if cfg!(target_os = "macos") {
                VideoCodec::Hevc
            } else {
                VideoCodec::H264
            },
            bitrate_mbps: 10,
            videotoolbox_speed_priority: true,
        }
    }
}

impl VideoEncodingSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=100).contains(&self.bitrate_mbps) {
            return Err("目标视频码率须在 1–100 Mbps 之间".into());
        }
        Ok(())
    }

    pub fn bitrate_kbps(&self, override_kbps: Option<NonZeroU32>) -> u64 {
        override_kbps.map_or(u64::from(self.bitrate_mbps) * 1000, |rate| {
            u64::from(rate.get())
        })
    }

    pub async fn encoder(&self) -> Result<AvailableVideoEncoder, String> {
        self.validate()?;
        available_encoders()
            .await?
            .iter()
            .find(|item| item.codec == self.codec)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "当前环境没有可用的 {:?} 编码器，请在设置中选择可用格式",
                    self.codec
                )
            })
    }

    pub fn apply(
        &self,
        command: &mut tokio::process::Command,
        encoder: &AvailableVideoEncoder,
        bitrate: Option<NonZeroU32>,
    ) {
        super::hwaccel::apply_x264_encoder_only(command, &encoder.encoder);
        command.args([
            "-b:v",
            &format!("{}k", self.bitrate_kbps(bitrate)),
            "-g",
            "120",
            "-bf",
            "0",
        ]);
        if encoder.videotoolbox {
            command.args([
                "-allow_sw",
                "0",
                "-prio_speed",
                if self.videotoolbox_speed_priority {
                    "1"
                } else {
                    "0"
                },
            ]);
        } else if encoder.encoder == "libx264" || encoder.encoder == "libx265" {
            command.args(["-preset", "medium"]);
        }
        if self.codec == VideoCodec::Hevc {
            command.args(["-tag:v", "hvc1"]);
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AvailableVideoEncoder {
    pub codec: VideoCodec,
    pub encoder: String,
    pub videotoolbox: bool,
}

static ENCODERS: OnceCell<Vec<AvailableVideoEncoder>> = OnceCell::const_new();

pub async fn available_encoders() -> Result<&'static Vec<AvailableVideoEncoder>, String> {
    ENCODERS
        .get_or_try_init(|| async {
            let mut result = Vec::new();
            #[cfg(target_os = "macos")]
            {
                let (h264, hevc) = tokio::task::spawn_blocking(metal_export::videotoolbox_codecs)
                    .await
                    .map_err(|e| e.to_string())??;
                for (codec, available, encoder) in [
                    (VideoCodec::H264, h264, "h264_videotoolbox"),
                    (VideoCodec::Hevc, hevc, "hevc_videotoolbox"),
                ] {
                    if available {
                        result.push(AvailableVideoEncoder {
                            codec,
                            encoder: encoder.into(),
                            videotoolbox: true,
                        });
                    }
                }
            }
            for (codec, candidates) in [
                (
                    VideoCodec::H264,
                    [
                        "h264_nvenc",
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
                        "hevc_qsv",
                        "hevc_amf",
                        "hevc_mf",
                        "hevc_vaapi",
                        "hevc_v4l2m2m",
                        "libx265",
                    ],
                ),
            ] {
                if result.iter().any(|item| item.codec == codec) {
                    continue;
                }
                for encoder in candidates {
                    if super::hwaccel::test_encoder_availability(encoder).await {
                        result.push(AvailableVideoEncoder {
                            codec,
                            encoder: encoder.into(),
                            videotoolbox: false,
                        });
                        break;
                    }
                }
            }
            Ok(result)
        })
        .await
}
