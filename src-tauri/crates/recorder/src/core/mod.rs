use std::fmt;
pub mod hls_recorder;
pub mod flv_recorder;
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

        if let Some(resolved) = self.resolve_yximgs_segment(seg_name, Some(&self.extra)) {
            return resolved;
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

    pub fn ts_url_without_extra(&self, seg_name: &str) -> String {
        if seg_name.starts_with("http://") || seg_name.starts_with("https://") {
            return seg_name.to_string();
        }

        if let Some(resolved) = self.resolve_yximgs_segment(seg_name, None) {
            return resolved;
        }

        let base = self.base.clone();
        let m3u8_filename = base.split('/').next_back().unwrap_or("");
        let base_url = if m3u8_filename.is_empty() {
            base
        } else {
            base.replace(m3u8_filename, seg_name)
        };
        let base_without_query = base_url.trim_end_matches('?');
        format!("{}{}", self.host, base_without_query)
    }

    fn resolve_yximgs_segment(&self, seg_name: &str, extra: Option<&str>) -> Option<String> {
        let (seg_path, seg_query) = seg_name.split_once('?').unwrap_or((seg_name, ""));
        let marker = ".yximgs.com_";
        let marker_index = seg_path.find(marker)?;
        let host_end = marker_index + ".yximgs.com".len();
        let host = &seg_path[..host_end];
        let rest = &seg_path[marker_index + marker.len()..];

        let base_path = self.base.split('?').next().unwrap_or(&self.base);
        let base_dir = base_path
            .rsplit_once('/')
            .map(|(dir, _)| dir)
            .unwrap_or("");
        let mut url = if base_dir.is_empty() {
            format!("https://{host}/")
        } else if base_dir.ends_with('/') {
            format!("https://{host}{base_dir}")
        } else {
            format!("https://{host}{base_dir}/")
        };
        url.push_str(rest);

        let mut query_parts = Vec::new();
        if !seg_query.is_empty() {
            query_parts.push(seg_query.to_string());
        } else if let Some(extra) = extra {
            let trimmed = extra.trim();
            if !trimmed.is_empty() {
                query_parts.push(trimmed.to_string());
            }
        }

        if !query_parts.is_empty() {
            url.push('?');
            url.push_str(&query_parts.join("&"));
        }

        Some(url)
    }

    pub fn is_expired(&self) -> bool {
        self.expire > 0 && (self.expire < chrono::Utc::now().timestamp() + SAFE_EXPIRE)
    }
}
