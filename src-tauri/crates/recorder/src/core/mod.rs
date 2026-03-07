use std::fmt;
pub mod flv_recorder;
pub mod hls_recorder;
pub mod playlist;
pub mod stream_info;

pub use stream_info::{
    CdnNode, Codec as StreamCodec, Format as StreamFormat, PlatformStreamInfo, PlatformType,
    Quality, RecorderType, StreamVariant,
};

#[derive(Clone, Debug, PartialEq)]
pub enum Format {
    Flv,
    TS,
    FMP4,
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Codec {
    Avc,
    Hevc,
}

impl fmt::Display for Codec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// A trait for HLS streams
///
/// This trait provides a common interface for HLS streams.
/// For example:
/// ```text
/// host: https://d1--cn-gotcha104b.bilivideo.com
/// base: /live-bvc/375028/live_2124647716_1414766_bluray.m3u8?
/// extra: expire=1734567890&oi=1234567890&s=1234567890&pt=0&ps=0&bw=1000000&tk=1234567890
/// ```
#[derive(Debug, Clone)]
pub struct HlsStream {
    id: String,
    host: String,
    base: String,
    extra: String,
    format: Format,
    codec: Codec,
    expire: i64,
}

const SAFE_EXPIRE: i64 = 3 * 60;

impl HlsStream {
    pub fn new(
        id: String,
        host: String,
        base: String,
        extra: String,
        format: Format,
        codec: Codec,
        expire: i64,
    ) -> Self {
        Self {
            id,
            host,
            base,
            extra,
            format,
            codec,
            expire,
        }
    }

    pub fn index(&self) -> String {
        if self.extra.is_empty() {
            format!("{}{}", self.host, self.base)
        } else {
            format!("{}{}{}", self.host, self.base, self.extra)
        }
    }

    pub fn ts_url(&self, seg_name: &str) -> String {
        // According to HLS spec (RFC 8216):
        // - If segment URI is absolute, use it directly
        // - If segment URI is relative, resolve it relative to the m3u8 base URL
        // - If segment URI contains query parameters, use them as-is (don't merge with m3u8 params)
        // - If segment URI doesn't contain query parameters, may add m3u8 params (non-standard but needed for some platforms)

        // Check if segment URI is absolute
        if seg_name.starts_with("http://") || seg_name.starts_with("https://") {
            return seg_name.to_string();
        }

        // Segment URI is relative, resolve it relative to m3u8 base URL
        let base = self.base.clone();
        let m3u8_filename = base.split('/').next_back().unwrap();
        let base_url = base.replace(m3u8_filename, seg_name);

        // Check if seg_name already contains query parameters
        if seg_name.contains('?') {
            // Segment URI already has query parameters, use it directly per HLS spec
            // Remove trailing '?' from base_url if present (from m3u8 base)
            let base_without_query = base_url.trim_end_matches('?');
            format!("{}{}", self.host, base_without_query)
        } else if self.extra.is_empty() {
            // No query parameters to add
            format!("{}{}", self.host, base_url)
        } else {
            // Segment URI has no query parameters, add m3u8 query parameters
            // (Non-standard but needed for some platforms like Huya)
            // Remove trailing '?' from base_url if present
            let base_without_query = base_url.trim_end_matches('?');
            format!("{}{}?{}", self.host, base_without_query, self.extra)
        }
    }

    pub fn is_expired(&self) -> bool {
        self.expire > 0 && (self.expire < chrono::Utc::now().timestamp() + SAFE_EXPIRE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_stream(host: &str, base: &str, extra: &str, expire: i64) -> HlsStream {
        HlsStream::new(
            "test_live".to_string(),
            host.to_string(),
            base.to_string(),
            extra.to_string(),
            Format::TS,
            Codec::Avc,
            expire,
        )
    }

    #[test]
    fn test_hls_stream_index_with_extra() {
        let s = make_stream(
            "https://cdn.example.com",
            "/live/stream.m3u8?",
            "expire=999&token=abc",
            0,
        );
        assert_eq!(
            s.index(),
            "https://cdn.example.com/live/stream.m3u8?expire=999&token=abc"
        );
    }

    #[test]
    fn test_hls_stream_index_without_extra() {
        let s = make_stream("https://cdn.example.com", "/live/stream.m3u8", "", 0);
        assert_eq!(s.index(), "https://cdn.example.com/live/stream.m3u8");
    }

    #[test]
    fn test_ts_url_absolute() {
        let s = make_stream(
            "https://cdn.example.com",
            "/live/stream.m3u8?",
            "token=abc",
            0,
        );
        let url = s.ts_url("https://other.com/seg001.ts");
        assert_eq!(url, "https://other.com/seg001.ts");
    }

    #[test]
    fn test_ts_url_relative_no_query() {
        let s = make_stream(
            "https://cdn.example.com",
            "/live/stream.m3u8?",
            "token=abc",
            0,
        );
        let url = s.ts_url("seg001.ts");
        assert_eq!(url, "https://cdn.example.com/live/seg001.ts?token=abc");
    }

    #[test]
    fn test_ts_url_relative_with_query() {
        let s = make_stream(
            "https://cdn.example.com",
            "/live/stream.m3u8?",
            "token=abc",
            0,
        );
        let url = s.ts_url("seg001.ts?own_token=xyz");
        assert_eq!(url, "https://cdn.example.com/live/seg001.ts?own_token=xyz");
    }

    #[test]
    fn test_ts_url_relative_no_extra() {
        let s = make_stream("https://cdn.example.com", "/live/stream.m3u8", "", 0);
        let url = s.ts_url("seg001.ts");
        assert_eq!(url, "https://cdn.example.com/live/seg001.ts");
    }

    #[test]
    fn test_is_expired_zero_expire() {
        let s = make_stream("https://cdn.example.com", "/live/stream.m3u8", "", 0);
        assert!(!s.is_expired());
    }

    #[test]
    fn test_is_expired_past() {
        let s = make_stream("https://cdn.example.com", "/live/stream.m3u8", "", 1000);
        assert!(s.is_expired());
    }

    #[test]
    fn test_is_expired_far_future() {
        let far_future = chrono::Utc::now().timestamp() + 100000;
        let s = make_stream(
            "https://cdn.example.com",
            "/live/stream.m3u8",
            "",
            far_future,
        );
        assert!(!s.is_expired());
    }

    #[test]
    fn test_format_display() {
        assert_eq!(format!("{}", Format::Flv), "Flv");
        assert_eq!(format!("{}", Format::TS), "TS");
        assert_eq!(format!("{}", Format::FMP4), "FMP4");
    }

    #[test]
    fn test_codec_display() {
        assert_eq!(format!("{}", Codec::Avc), "Avc");
        assert_eq!(format!("{}", Codec::Hevc), "Hevc");
    }

    #[test]
    fn test_format_equality() {
        assert_eq!(Format::Flv, Format::Flv);
        assert_ne!(Format::Flv, Format::TS);
    }

    #[test]
    fn test_codec_equality() {
        assert_eq!(Codec::Avc, Codec::Avc);
        assert_ne!(Codec::Avc, Codec::Hevc);
    }
}
