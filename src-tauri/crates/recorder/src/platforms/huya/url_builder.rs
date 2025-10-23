use std::collections::HashMap;
use url::Url;

/// 播放器配置信息
#[derive(Debug, Clone)]
pub struct PlayerInfo {
    /// 解码后的基础URL
    pub url: String,
    /// 流名称
    pub s_stream_name: Option<String>,
    /// 主播UID
    pub presenter_uid: Option<String>,
    /// HLS防码参数
    pub s_hls_anti_code: Option<String>,
}

/// URL构建器
pub struct UrlBuilder;

impl UrlBuilder {
    fn generate_uid() -> u64 {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let random = fastrand::u32(0..1000000);
        timestamp as u64 * 1000 + random as u64
    }

    fn generate_s_guid() -> String {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let random = fastrand::u32(0..1000000);
        format!("{}_{}", timestamp, random)
    }

    /// 构建播放器URL
    ///
    /// # Arguments
    /// * `info` - 播放器配置信息
    ///
    /// # Returns
    /// * `Result<String, String>` - 完整的播放URL或错误信息
    pub fn build_player_url(info: &PlayerInfo) -> Result<String, String> {
        if info.url.is_empty() {
            return Err("URL is required".to_string());
        }

        let mut base_url = info.url.clone();

        // 确保URL以?开头，如果没有则添加
        if !base_url.contains('?') {
            base_url.push('?');
        } else if !base_url.ends_with('&') && !base_url.ends_with('?') {
            base_url.push('&');
        }

        // 添加HLS防码参数
        if let Some(anti_code) = &info.s_hls_anti_code {
            base_url.push_str(anti_code);
        }

        // 添加用户身份参数
        base_url.push_str(&format!("&uid={}", Self::generate_uid()));
        base_url.push_str(&format!("&sGuid={}", Self::generate_s_guid()));
        base_url.push_str(&format!("&appid={}", 66));

        // 添加流信息参数
        if let Some(s_stream_name) = &info.s_stream_name {
            base_url.push_str(&format!(
                "&sStreamName={}",
                urlencoding::encode(s_stream_name)
            ));
        }

        if let Some(presenter_uid) = &info.presenter_uid {
            base_url.push_str(&format!(
                "&presenterUid={}",
                urlencoding::encode(presenter_uid)
            ));
        }

        // 添加播放配置参数
        base_url.push_str(&format!("&playTimeout={}", 5000));
        base_url.push_str(&format!(
            "&h5Root={}",
            "https://hd.huya.com/cdn_libs/mobile/"
        ));

        // 添加动态参数
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        base_url.push_str(&format!("&t={}", timestamp));

        // 生成序列ID
        let seq_id = Self::generate_seq_id();
        base_url.push_str(&format!("&seqId={}", seq_id));

        // 添加其他必要参数
        base_url.push_str("&ver=1");
        base_url.push_str(&format!("&sv={}", Self::get_version()));

        Ok(base_url)
    }

    /// 解析虎牙直播URL参数
    ///
    /// # Arguments
    /// * `url` - 完整的播放URL
    ///
    /// # Returns
    /// * `Result<(String, HashMap<String, String>), String>` - 基础URL和参数映射
    pub fn parse_player_url(url: &str) -> Result<(String, HashMap<String, String>), String> {
        let url_obj = Url::parse(url).map_err(|e| format!("Failed to parse URL: {}", e))?;
        let mut params = HashMap::new();

        for (key, value) in url_obj.query_pairs() {
            params.insert(key.to_string(), value.to_string());
        }

        let base_url = format!(
            "{}://{}{}",
            url_obj.scheme(),
            url_obj.host_str().unwrap_or(""),
            url_obj.path()
        );
        Ok((base_url, params))
    }

    /// 验证播放URL是否有效
    ///
    /// # Arguments
    /// * `url` - 播放URL
    ///
    /// # Returns
    /// * `bool` - 是否有效
    pub fn validate_player_url(url: &str) -> bool {
        match Url::parse(url) {
            Ok(url_obj) => {
                let params: HashMap<String, String> = url_obj
                    .query_pairs()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();

                // 检查必需参数
                let required_params = ["uid", "sGuid", "appid", "seqId", "t"];
                required_params
                    .iter()
                    .all(|param| params.contains_key(*param))
            }
            Err(_) => false,
        }
    }

    /// 生成序列ID
    /// 模拟播放器内部的getAnticodeSeqid()方法
    ///
    /// # Returns
    /// * `String` - 序列ID
    fn generate_seq_id() -> String {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let random = fastrand::u32(0..1000000);
        format!("{}_{}", timestamp, random)
    }

    /// 获取版本号
    /// 模拟播放器内部的版本获取逻辑
    ///
    /// # Returns
    /// * `String` - 版本号
    fn get_version() -> String {
        let now = chrono::Utc::now();
        now.format("%Y%m%d%H%M").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_player_url() {
        let info = PlayerInfo {
            url: "https://tx.hls.huya.com/src/431653844-431653844-1853939143172685824-863431144-10057-A-0-1-imgplus.m3u8?ratio=2000&wsSecret=725304fc2867cbe6254f12b264055136&wsTime=68fb9aa9&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103".to_string(),
            s_stream_name: Some("431653844-431653844-1853939143172685824-863431144-10057-A-0-1-imgplus".to_string()),
            presenter_uid: Some("431653844".to_string()),
            s_hls_anti_code: Some("wsSecret=820369d885b161baa5a7a82170881d78&wsTime=68fb97be&fm=RFdxOEJjSjNoNkRKdDZUWV8kMF8kMV8kMl8kMw%3D%3D&ctype=tars_mobile&fs=bgct&t=103".to_string()),
        };

        let result = UrlBuilder::build_player_url(&info);
        assert!(result.is_ok());
        let url = result.unwrap();
        println!("url: {}", url);
        assert!(url.contains("appid=66"));
        assert!(url.contains("seqId="));
        assert!(url.contains("t="));
    }

    #[test]
    fn test_validate_player_url() {
        let valid_url =
            "https://example.com/stream.m3u8?uid=123&sGuid=abc&appid=66&seqId=123_456&t=1234567890";
        assert!(UrlBuilder::validate_player_url(valid_url));

        let invalid_url = "https://example.com/stream.m3u8?uid=123&sGuid=abc";
        assert!(!UrlBuilder::validate_player_url(invalid_url));
    }
}
