use serde_derive::Deserialize;
use serde_derive::Serialize;
use serde_json::Value;

use crate::core::stream_info::{
    CdnNode, Codec, Format, PlatformStreamInfo, PlatformType, Quality, StreamVariant,
};
use crate::errors::RecorderError;

/// A pull URL extracted from Douyin `stream_data`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableStream {
    pub quality: String,
    pub variant: String,
    pub format: &'static str,
    pub url: String,
}

impl AvailableStream {
    pub fn label(&self) -> String {
        format!("{}/{} {}", self.quality, self.variant, self.format)
    }
}

const QUALITY_ORDER: &[&str] = &["origin", "uhd", "hd", "sd", "ld", "md", "ao"];
const VARIANT_ORDER: &[&str] = &["main", "backup"];
const FORMAT_ORDER: &[&str] = &["hls", "flv"];

/// Collect all non-empty HLS/FLV pull URLs from Douyin `stream_data` JSON.
pub fn collect_available_streams(stream_data: &str) -> Vec<AvailableStream> {
    let Ok(value) = serde_json::from_str::<Value>(stream_data) else {
        return Vec::new();
    };
    collect_available_streams_from_value(&value)
}

fn collect_available_streams_from_value(value: &Value) -> Vec<AvailableStream> {
    let qualities = value
        .get("data")
        .and_then(Value::as_object)
        .or_else(|| value.as_object());
    let Some(qualities) = qualities else {
        return Vec::new();
    };

    let mut streams = Vec::new();
    for (quality, quality_val) in qualities {
        let Some(variants) = quality_val.as_object() else {
            continue;
        };
        for (variant, variant_val) in variants {
            let Some(urls) = variant_val.as_object() else {
                continue;
            };
            for format in FORMAT_ORDER {
                if let Some(url) = urls.get(*format).and_then(Value::as_str) {
                    if !url.is_empty() {
                        streams.push(AvailableStream {
                            quality: quality.clone(),
                            variant: variant.clone(),
                            format,
                            url: url.to_string(),
                        });
                    }
                }
            }
        }
    }

    streams.sort_by(|a, b| {
        rank(QUALITY_ORDER, &a.quality)
            .cmp(&rank(QUALITY_ORDER, &b.quality))
            .then_with(|| a.quality.cmp(&b.quality))
            .then_with(|| rank(VARIANT_ORDER, &a.variant).cmp(&rank(VARIANT_ORDER, &b.variant)))
            .then_with(|| a.variant.cmp(&b.variant))
            .then_with(|| rank(FORMAT_ORDER, a.format).cmp(&rank(FORMAT_ORDER, b.format)))
    });
    streams
}

fn rank(order: &[&str], key: &str) -> usize {
    order
        .iter()
        .position(|item| *item == key)
        .unwrap_or(order.len())
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DouyinStream {
    pub data: Data,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Data {
    pub origin: Origin,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct Ld {
    pub main: Main,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Main {
    pub flv: String,
    pub hls: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct Md {
    pub main: Main,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Origin {
    pub main: Main,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct Sd {
    pub main: Main,
}
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct Hd {
    pub main: Main,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct Ao {
    pub main: Main,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct Uhd {
    pub main: Main,
}

// 实现 PlatformStreamInfo trait
impl PlatformStreamInfo for DouyinStream {
    fn primary_variant(&self) -> Result<StreamVariant, RecorderError> {
        Ok(StreamVariant {
            url: self.data.origin.main.hls.clone(),
            format: Format::HLS,
            codec: Codec::AVC,
            quality: Quality::Origin,
            bitrate: None,
        })
    }

    fn all_variants(&self) -> Vec<StreamVariant> {
        match self.primary_variant() {
            Ok(variant) => vec![variant],
            Err(e) => {
                log::warn!("Failed to build primary stream variant: {e}");
                Vec::new()
            }
        }
    }

    fn expires_at(&self) -> Option<i64> {
        None // Douyin 流不过期
    }

    fn cdn_nodes(&self) -> Vec<CdnNode> {
        Vec::new() // Douyin 单 CDN
    }

    fn platform(&self) -> PlatformType {
        PlatformType::Douyin
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_available_streams_lists_hls_and_flv_by_quality() {
        let stream_data = r#"{
            "common": {"session_id": "abc"},
            "data": {
                "hd": {
                    "main": {
                        "flv": "http://cdn/hd.flv",
                        "hls": "http://cdn/hd.m3u8"
                    }
                },
                "origin": {
                    "main": {
                        "flv": "http://cdn/origin.flv",
                        "hls": "http://cdn/origin.m3u8"
                    },
                    "backup": {
                        "hls": "http://cdn/origin-backup.m3u8",
                        "flv": ""
                    }
                }
            }
        }"#;

        let streams = collect_available_streams(stream_data);
        let labels: Vec<_> = streams.iter().map(AvailableStream::label).collect();
        assert_eq!(
            labels,
            vec![
                "origin/main hls",
                "origin/main flv",
                "origin/backup hls",
                "hd/main hls",
                "hd/main flv",
            ]
        );
        assert_eq!(streams[0].url, "http://cdn/origin.m3u8");
    }

    #[test]
    fn collect_available_streams_accepts_inner_data_object() {
        let stream_data = r#"{
            "origin": {
                "main": { "hls": "http://cdn/origin.m3u8", "flv": "http://cdn/origin.flv" }
            }
        }"#;

        let streams = collect_available_streams(stream_data);
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].label(), "origin/main hls");
    }

    #[test]
    fn collect_available_streams_returns_empty_for_invalid_json() {
        assert!(collect_available_streams("not-json").is_empty());
    }
}
