use serde_derive::Deserialize;
use serde_derive::Serialize;

use crate::core::stream_info::{
    CdnNode, Codec, Format, PlatformStreamInfo, PlatformType, Quality, StreamVariant,
};
use crate::errors::RecorderError;

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
        vec![self.primary_variant().unwrap()]
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
