use std::fmt;
pub mod hls_recorder;
pub mod playlist;

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
}

impl HlsStream {
    pub fn new(
        id: String,
        host: String,
        base: String,
        extra: String,
        format: Format,
        codec: Codec,
    ) -> Self {
        Self {
            id,
            host,
            base,
            extra,
            format,
            codec,
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
        let base = self.base.clone();
        let m3u8_filename = base.split('/').next_back().unwrap();
        let base_url = base.replace(m3u8_filename, seg_name);
        if self.extra.is_empty() {
            format!("{}{}", self.host, base_url)
        } else {
            // Check if base_url already contains query parameters
            if base_url.contains('?') {
                // If seg_name already has query params, append extra with '&'
                // Remove trailing '?' or '&' before appending
                let base_trimmed = base_url.trim_end_matches('?').trim_end_matches('&');
                format!("{}{}&{}", self.host, base_trimmed, self.extra)
            } else {
                // If no query params, add them with '?'
                // Remove trailing '?' from base_url if present
                let base_without_query = base_url.trim_end_matches('?');
                format!("{}{}?{}", self.host, base_without_query, self.extra)
            }
        }
    }
}
