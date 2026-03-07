use crate::core::{Codec as CoreCodec, Format as CoreFormat, HlsStream};
use crate::errors::RecorderError;
use std::fmt::Debug;
use std::sync::Arc;

/// 跨平台质量等级枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Quality {
    /// 原画
    Origin,
    /// 4K 蓝光
    BluRay4K,
    /// 蓝光
    BluRay,
    /// 超清
    UltraHD,
    /// 高清
    HD,
    /// 标清
    SD,
    /// 流畅
    Smooth,
}

/// 流格式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    HLS,
    FLV,
    RTMP,
}

/// 编码格式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    AVC,
    HEVC,
}

/// CDN 节点信息
#[derive(Debug, Clone)]
pub struct CdnNode {
    pub host: String,
    pub priority: u8,
}

/// 统一的流变体表示
#[derive(Debug, Clone)]
pub struct StreamVariant {
    pub url: String,
    pub format: Format,
    pub codec: Codec,
    pub quality: Quality,
    pub bitrate: Option<u64>,
}

impl StreamVariant {
    /// 转换为 HlsStream
    /// 注意：此方法需要 URL 已经是完整的 HLS URL
    pub fn to_hls_stream(
        &self,
        live_id: String,
        cdn_node: Option<&CdnNode>,
    ) -> Result<HlsStream, RecorderError> {
        if self.format != Format::HLS {
            return Err(RecorderError::ApiError {
                error: "Stream is not HLS format".to_string(),
            });
        }

        let url = if let Some(node) = cdn_node {
            // 替换 URL 中的主机为指定 CDN 节点
            self.url.replace(&extract_host(&self.url)?, &node.host)
        } else {
            self.url.clone()
        };

        // 解析 URL 为 HlsStream 组件
        let parsed = url::Url::parse(&url).map_err(|e| RecorderError::ApiError {
            error: format!("Invalid URL: {}", e),
        })?;

        let host = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""));
        let base = parsed.path().to_string();
        let extra = parsed.query().unwrap_or("").to_string();

        // 从 URL 中提取过期时间（如果有）
        let expire = parsed
            .query_pairs()
            .find(|(k, _)| k == "expire")
            .and_then(|(_, v)| v.parse::<i64>().ok())
            .unwrap_or(0);

        // 转换格式和编码
        let core_format = match self.codec {
            Codec::AVC => CoreFormat::TS,
            Codec::HEVC => CoreFormat::FMP4,
        };
        let core_codec = match self.codec {
            Codec::AVC => CoreCodec::Avc,
            Codec::HEVC => CoreCodec::Hevc,
        };

        Ok(HlsStream::new(
            live_id,
            host,
            base,
            extra,
            core_format,
            core_codec,
            expire,
        ))
    }

    /// 获取 FLV URL
    pub fn to_flv_url(&self) -> Result<String, RecorderError> {
        if self.format != Format::FLV && self.format != Format::RTMP {
            return Err(RecorderError::ApiError {
                error: "Stream is not FLV or RTMP format".to_string(),
            });
        }
        Ok(self.url.clone())
    }

    /// 自动选择 Recorder 类型
    pub fn to_recorder_type(
        &self,
        live_id: String,
        cdn_node: Option<&CdnNode>,
    ) -> Result<RecorderType, RecorderError> {
        match self.format {
            Format::HLS => {
                let stream = self.to_hls_stream(live_id, cdn_node)?;
                Ok(RecorderType::Hls(Arc::new(stream)))
            }
            Format::FLV | Format::RTMP => {
                let url = self.to_flv_url()?;
                Ok(RecorderType::Flv(url))
            }
        }
    }
}

/// Recorder 类型
#[derive(Debug, Clone)]
pub enum RecorderType {
    Hls(Arc<HlsStream>),
    Flv(String),
}

/// 平台类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformType {
    Bilibili,
    Douyin,
    Kuaishou,
    Huya,
    TikTok,
}

impl PlatformType {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlatformType::Bilibili => "bilibili",
            PlatformType::Douyin => "douyin",
            PlatformType::Kuaishou => "kuaishou",
            PlatformType::Huya => "huya",
            PlatformType::TikTok => "tiktok",
        }
    }
}

/// 所有平台流信息必须实现的核心 trait
pub trait PlatformStreamInfo: Clone + Send + Sync + Debug {
    /// 获取主流变体（最高质量）
    fn primary_variant(&self) -> Result<StreamVariant, RecorderError>;

    /// 获取所有可用流变体
    fn all_variants(&self) -> Vec<StreamVariant>;

    /// 获取过期时间戳（Unix 时间戳，秒）
    fn expires_at(&self) -> Option<i64>;

    /// 获取 CDN 节点列表
    fn cdn_nodes(&self) -> Vec<CdnNode>;

    /// 获取平台类型
    fn platform(&self) -> PlatformType;

    /// 检查是否过期
    fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            now >= expires_at
        } else {
            false
        }
    }
}

/// 辅助函数：从 URL 提取主机
fn extract_host(url: &str) -> Result<String, RecorderError> {
    url::Url::parse(url)
        .map(|u| u.host_str().unwrap_or("").to_string())
        .map_err(|e| RecorderError::ApiError {
            error: format!("Invalid URL: {}", e),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quality_ordering() {
        assert!(Quality::Origin < Quality::BluRay4K);
        assert!(Quality::BluRay4K < Quality::BluRay);
        assert!(Quality::BluRay < Quality::UltraHD);
        assert!(Quality::UltraHD < Quality::HD);
        assert!(Quality::HD < Quality::SD);
        assert!(Quality::SD < Quality::Smooth);
    }

    #[test]
    fn test_stream_variant_to_flv_url_ok() {
        let sv = StreamVariant {
            url: "rtmp://live.example.com/stream".to_string(),
            format: Format::FLV,
            codec: Codec::AVC,
            quality: Quality::Origin,
            bitrate: Some(5000),
        };
        assert_eq!(sv.to_flv_url().unwrap(), "rtmp://live.example.com/stream");
    }

    #[test]
    fn test_stream_variant_to_flv_url_rtmp() {
        let sv = StreamVariant {
            url: "rtmp://live.example.com/stream".to_string(),
            format: Format::RTMP,
            codec: Codec::AVC,
            quality: Quality::Origin,
            bitrate: None,
        };
        assert!(sv.to_flv_url().is_ok());
    }

    #[test]
    fn test_stream_variant_to_flv_url_hls_fails() {
        let sv = StreamVariant {
            url: "https://cdn.example.com/live/stream.m3u8".to_string(),
            format: Format::HLS,
            codec: Codec::AVC,
            quality: Quality::Origin,
            bitrate: None,
        };
        assert!(sv.to_flv_url().is_err());
    }

    #[test]
    fn test_stream_variant_to_hls_stream() {
        let sv = StreamVariant {
            url: "https://cdn.example.com/live/stream.m3u8?expire=9999999999&token=abc".to_string(),
            format: Format::HLS,
            codec: Codec::AVC,
            quality: Quality::BluRay,
            bitrate: Some(3000),
        };
        let stream = sv.to_hls_stream("live_123".to_string(), None).unwrap();
        let index = stream.index();
        assert!(index.contains("cdn.example.com"));
        assert!(index.contains("stream.m3u8"));
    }

    #[test]
    fn test_stream_variant_to_hls_stream_not_hls() {
        let sv = StreamVariant {
            url: "rtmp://live.example.com/stream".to_string(),
            format: Format::FLV,
            codec: Codec::AVC,
            quality: Quality::Origin,
            bitrate: None,
        };
        assert!(sv.to_hls_stream("live_123".to_string(), None).is_err());
    }

    #[test]
    fn test_stream_variant_to_hls_stream_with_cdn_node() {
        let sv = StreamVariant {
            url: "https://cdn1.example.com/live/stream.m3u8?token=abc".to_string(),
            format: Format::HLS,
            codec: Codec::AVC,
            quality: Quality::Origin,
            bitrate: None,
        };
        let cdn = CdnNode {
            host: "cdn2.example.com".to_string(),
            priority: 1,
        };
        let stream = sv
            .to_hls_stream("live_123".to_string(), Some(&cdn))
            .unwrap();
        let index = stream.index();
        assert!(index.contains("cdn2.example.com"));
    }

    #[test]
    fn test_stream_variant_to_recorder_type_hls() {
        let sv = StreamVariant {
            url: "https://cdn.example.com/live/stream.m3u8?token=abc".to_string(),
            format: Format::HLS,
            codec: Codec::AVC,
            quality: Quality::Origin,
            bitrate: None,
        };
        let rt = sv.to_recorder_type("live_123".to_string(), None).unwrap();
        assert!(matches!(rt, RecorderType::Hls(_)));
    }

    #[test]
    fn test_stream_variant_to_recorder_type_flv() {
        let sv = StreamVariant {
            url: "rtmp://live.example.com/stream".to_string(),
            format: Format::FLV,
            codec: Codec::AVC,
            quality: Quality::Origin,
            bitrate: None,
        };
        let rt = sv.to_recorder_type("live_123".to_string(), None).unwrap();
        assert!(matches!(rt, RecorderType::Flv(_)));
    }

    #[test]
    fn test_platform_type_as_str() {
        assert_eq!(PlatformType::Bilibili.as_str(), "bilibili");
        assert_eq!(PlatformType::Douyin.as_str(), "douyin");
        assert_eq!(PlatformType::Kuaishou.as_str(), "kuaishou");
        assert_eq!(PlatformType::Huya.as_str(), "huya");
        assert_eq!(PlatformType::TikTok.as_str(), "tiktok");
    }

    #[test]
    fn test_extract_host() {
        assert_eq!(
            extract_host("https://cdn.example.com/path").unwrap(),
            "cdn.example.com"
        );
        assert!(extract_host("not-a-url").is_err());
    }

    #[test]
    fn test_quality_equality() {
        assert_eq!(Quality::Origin, Quality::Origin);
        assert_ne!(Quality::Origin, Quality::HD);
    }

    #[test]
    fn test_format_equality() {
        assert_eq!(Format::HLS, Format::HLS);
        assert_ne!(Format::HLS, Format::FLV);
    }

    #[test]
    fn test_codec_equality() {
        assert_eq!(Codec::AVC, Codec::AVC);
        assert_ne!(Codec::AVC, Codec::HEVC);
    }
}
