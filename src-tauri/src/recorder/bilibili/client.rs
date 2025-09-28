use super::errors::BiliClientError;
use super::profile;
use super::profile::Profile;
use super::response;
use super::response::GeneralResponse;
use super::response::PostVideoMetaResponse;
use super::response::PreuploadResponse;
use super::response::VideoSubmitData;
use crate::database::account::AccountRow;
use crate::progress::progress_reporter::ProgressReporter;
use crate::progress::progress_reporter::ProgressReporterTrait;
use crate::recorder::user_agent_generator;
use chrono::TimeZone;
use pct_str::PctString;
use pct_str::URIReserved;
use rand::seq::SliceRandom;
use regex::Regex;
use reqwest::Client;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use serde_json::Value;
use std::fmt;
use std::path::Path;
use std::time::Duration;
use std::time::SystemTime;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::time::Instant;

#[derive(Clone)]
struct UploadParams<'a> {
    reporter: &'a ProgressReporter,
    preupload_response: &'a PreuploadResponse,
    post_video_meta_response: &'a PostVideoMetaResponse,
    video_file: &'a Path,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RoomInfo {
    pub live_status: u8,
    pub room_cover_url: String,
    pub room_id: i64,
    pub room_keyframe_url: String,
    pub room_title: String,
    pub user_id: i64,
    pub live_start_time: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UserInfo {
    pub user_id: i64,
    pub user_name: String,
    pub user_sign: String,
    pub user_avatar_url: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QrInfo {
    pub oauth_key: String,
    pub url: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QrStatus {
    pub code: u8,
    pub cookies: String,
}

/// `BiliClient` is thread safe
pub struct BiliClient {
    client: Client,
}

#[derive(Clone, Debug)]
pub struct BiliStream {
    pub format: Format,
    pub codec: Codec,
    pub base_url: String,
    pub url_info: Vec<UrlInfo>,
    pub drm: bool,
    pub master_url: Option<String>,
}

#[derive(Clone, Debug)]
pub struct UrlInfo {
    pub host: String,
    pub extra: String,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum Protocol {
    HttpStream,
    HttpHls,
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

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

#[derive(Clone, Debug)]
pub enum Codec {
    Avc,
    Hevc,
}

impl fmt::Display for Codec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

// 30000	杜比
// 20000	4K
// 15000    2K
// 10000	原画
// 400	蓝光
// 250	超清
// 150	高清
// 80	流畅

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum Qn {
    Dolby = 30000,
    Q4K = 20000,
    Q2K = 15000,
    Q1080PH = 10000,
    Q1080P = 400,
    Q720P = 250,
    Hd = 150,
    Smooth = 80,
}

impl fmt::Display for Qn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl fmt::Display for BiliStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "type: {:?}, codec: {:?}, base_url: {}, url_info: {:?}, drm: {}, master_url: {:?}",
            self.format, self.codec, self.base_url, self.url_info, self.drm, self.master_url
        )
    }
}

impl BiliStream {
    pub fn new(
        format: Format,
        codec: Codec,
        base_url: &str,
        url_info: Vec<UrlInfo>,
        drm: bool,
        master_url: Option<String>,
    ) -> BiliStream {
        BiliStream {
            format,
            codec,
            base_url: base_url.into(),
            url_info,
            drm,
            master_url,
        }
    }

    pub fn index(&self) -> String {
        // random choose a url_info
        let url_info = self.url_info.choose(&mut rand::thread_rng()).unwrap();
        format!("{}{}{}", url_info.host, self.base_url, url_info.extra)
    }

    pub fn ts_url(&self, seg_name: &str) -> String {
        let m3u8_filename = self.base_url.split('/').next_back().unwrap();
        let base_url = self.base_url.replace(m3u8_filename, seg_name);
        let url_info = self.url_info.choose(&mut rand::thread_rng()).unwrap();
        format!("{}{}?{}", url_info.host, base_url, url_info.extra)
    }

    pub fn get_expire(&self) -> Option<i64> {
        let url_info = self.url_info.choose(&mut rand::thread_rng()).unwrap();
        url_info.extra.split('&').find_map(|param| {
            if param.starts_with("expires=") {
                param.split('=').nth(1)?.parse().ok()
            } else {
                None
            }
        })
    }
}

impl BiliClient {
    pub fn new() -> Result<BiliClient, BiliClientError> {
        if let Ok(client) = Client::builder().timeout(Duration::from_secs(10)).build() {
            Ok(BiliClient { client })
        } else {
            Err(BiliClientError::InitClientError)
        }
    }

    fn generate_user_agent_header(&self) -> reqwest::header::HeaderMap {
        let user_agent = user_agent_generator::UserAgentGenerator::new().generate();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("user-agent", user_agent.parse().unwrap());
        headers
    }

    pub async fn get_qr(&self) -> Result<QrInfo, BiliClientError> {
        let headers = self.generate_user_agent_header();
        let res: serde_json::Value = self
            .client
            .get("https://passport.bilibili.com/x/passport-login/web/qrcode/generate")
            .headers(headers)
            .send()
            .await?
            .json()
            .await?;
        Ok(QrInfo {
            oauth_key: res["data"]["qrcode_key"]
                .as_str()
                .ok_or(BiliClientError::InvalidValue)?
                .to_string(),
            url: res["data"]["url"]
                .as_str()
                .ok_or(BiliClientError::InvalidValue)?
                .to_string(),
        })
    }

    pub async fn get_qr_status(&self, qrcode_key: &str) -> Result<QrStatus, BiliClientError> {
        let headers = self.generate_user_agent_header();
        let res: serde_json::Value = self
            .client
            .get(format!(
                "https://passport.bilibili.com/x/passport-login/web/qrcode/poll?qrcode_key={qrcode_key}"
            ))
            .headers(headers)
            .send()
            .await?
            .json()
            .await?;
        let code: u8 = res["data"]["code"].as_u64().unwrap_or(400) as u8;
        let mut cookies: String = String::new();
        if code == 0 {
            let url = res["data"]["url"]
                .as_str()
                .ok_or(BiliClientError::InvalidValue)?
                .to_string();
            let query_str = url.split('?').next_back().unwrap();
            cookies = query_str.replace('&', ";");
        }
        Ok(QrStatus { code, cookies })
    }

    pub async fn logout(&self, account: &AccountRow) -> Result<(), BiliClientError> {
        let mut headers = self.generate_user_agent_header();
        let url = "https://passport.bilibili.com/login/exit/v2";
        if let Ok(cookies) = account.cookies.parse() {
            headers.insert("cookie", cookies);
        } else {
            return Err(BiliClientError::InvalidCookie);
        }
        let params = [("csrf", account.csrf.clone())];
        let _ = self
            .client
            .post(url)
            .headers(headers)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&params)
            .send()
            .await?;
        Ok(())
    }

    pub async fn get_user_info(
        &self,
        account: &AccountRow,
        user_id: i64,
    ) -> Result<UserInfo, BiliClientError> {
        let params: Value = json!({
            "mid": user_id.to_string(),
            "platform": "web",
            "web_location": "1550101",
            "token": "",
            "w_webid": "",
        });
        let params = self.get_sign(params).await?;
        let mut headers = self.generate_user_agent_header();
        if let Ok(cookies) = account.cookies.parse() {
            headers.insert("cookie", cookies);
        } else {
            return Err(BiliClientError::InvalidCookie);
        }
        let resp = self
            .client
            .get(format!(
                "https://api.bilibili.com/x/space/wbi/acc/info?{params}"
            ))
            .headers(headers)
            .send()
            .await?;

        if !resp.status().is_success() {
            if resp.status() == reqwest::StatusCode::PRECONDITION_FAILED {
                return Err(BiliClientError::SecurityControlError);
            }
            return Err(BiliClientError::InvalidResponseStatus {
                status: resp.status(),
            });
        }

        let res: serde_json::Value = resp.json().await?;
        let code = res["code"]
            .as_u64()
            .ok_or(BiliClientError::InvalidResponseJson { resp: res.clone() })?;
        if code != 0 {
            log::error!("Get user info failed {code}");
            return Err(BiliClientError::InvalidMessageCode { code });
        }
        Ok(UserInfo {
            user_id,
            user_name: res["data"]["name"].as_str().unwrap_or("").to_string(),
            user_sign: res["data"]["sign"].as_str().unwrap_or("").to_string(),
            user_avatar_url: res["data"]["face"].as_str().unwrap_or("").to_string(),
        })
    }

    pub async fn get_room_info(
        &self,
        account: &AccountRow,
        room_id: i64,
    ) -> Result<RoomInfo, BiliClientError> {
        let mut headers = self.generate_user_agent_header();
        if let Ok(cookies) = account.cookies.parse() {
            headers.insert("cookie", cookies);
        } else {
            return Err(BiliClientError::InvalidCookie);
        }
        let response = self
            .client
            .get(format!(
                "https://api.live.bilibili.com/room/v1/Room/get_info?room_id={room_id}"
            ))
            .headers(headers)
            .send()
            .await?;

        if !response.status().is_success() {
            if response.status() == reqwest::StatusCode::PRECONDITION_FAILED {
                return Err(BiliClientError::SecurityControlError);
            }
            return Err(BiliClientError::InvalidResponseStatus {
                status: response.status(),
            });
        }

        let res: serde_json::Value = response.json().await?;
        let code = res["code"]
            .as_u64()
            .ok_or(BiliClientError::InvalidResponseJson { resp: res.clone() })?;
        if code != 0 {
            return Err(BiliClientError::InvalidMessageCode { code });
        }

        let room_id = res["data"]["room_id"]
            .as_i64()
            .ok_or(BiliClientError::InvalidValue)?;
        let room_title = res["data"]["title"]
            .as_str()
            .ok_or(BiliClientError::InvalidValue)?
            .to_string();
        let room_cover_url = res["data"]["user_cover"]
            .as_str()
            .ok_or(BiliClientError::InvalidValue)?
            .to_string();
        let room_keyframe_url = res["data"]["keyframe"]
            .as_str()
            .ok_or(BiliClientError::InvalidValue)?
            .to_string();
        let user_id = res["data"]["uid"]
            .as_i64()
            .ok_or(BiliClientError::InvalidValue)?;
        let live_status = res["data"]["live_status"]
            .as_u64()
            .ok_or(BiliClientError::InvalidValue)? as u8;
        // "live_time": "2025-08-09 18:33:35",
        let live_start_time_str = res["data"]["live_time"]
            .as_str()
            .ok_or(BiliClientError::InvalidValue)?;
        let live_start_time = if live_start_time_str == "0000-00-00 00:00:00" {
            0
        } else {
            // this is a fixed Asia/Shanghai datetime str
            let naive =
                chrono::NaiveDateTime::parse_from_str(live_start_time_str, "%Y-%m-%d %H:%M:%S")
                    .map_err(|_| BiliClientError::InvalidValue)?;
            // parse as UTC datetime and convert to timestamp
            chrono::Utc
                .from_local_datetime(&naive)
                .earliest()
                .ok_or(BiliClientError::InvalidValue)?
                .timestamp()
                - 8 * 3600
        };
        Ok(RoomInfo {
            live_status,
            room_cover_url,
            room_id,
            room_keyframe_url,
            room_title,
            user_id,
            live_start_time,
        })
    }

    /// Get stream info from room id
    ///
    /// https://socialsisteryi.github.io/bilibili-API-collect/docs/live/info.html#%E8%8E%B7%E5%8F%96%E7%9B%B4%E6%92%AD%E9%97%B4%E4%BF%A1%E6%81%AF-1
    /// https://api.live.bilibili.com/xlive/web-room/v2/index/getRoomPlayInfo?room_id=31368705&protocol=1&format=1&codec=0&qn=10000&platform=h5
    pub async fn get_stream_info(
        &self,
        account: &AccountRow,
        room_id: i64,
        protocol: Protocol,
        format: Format,
        codec: &[Codec],
        qn: Qn,
    ) -> Result<BiliStream, BiliClientError> {
        let url = format!(
            "https://api.live.bilibili.com/xlive/web-room/v2/index/getRoomPlayInfo?room_id={}&protocol={}&format={}&codec={}&qn={}&platform=h5",
            room_id,
            protocol.clone() as u8,
            format.clone() as u8,
            codec.iter().map(|c| (c.clone() as u8).to_string()).collect::<Vec<String>>().join(","),
            qn as i64,
        );
        let mut headers = self.generate_user_agent_header();
        if let Ok(cookies) = account.cookies.parse() {
            headers.insert("cookie", cookies);
        } else {
            return Err(BiliClientError::InvalidCookie);
        }
        let response = self.client.get(url).headers(headers).send().await?;
        let res: serde_json::Value = response.json().await?;

        let code = res["code"].as_u64().unwrap_or(0);
        let message = res["message"].as_str().unwrap_or("");
        if code != 0 {
            return Err(BiliClientError::ApiError(format!(
                "Code {} not found, message: {}",
                code, message
            )));
        }

        log::debug!("Get stream info response: {res}");

        // Parse the new API response structure
        let playurl_info = &res["data"]["playurl_info"]["playurl"];
        let empty_vec = vec![];
        let streams = playurl_info["stream"].as_array().unwrap_or(&empty_vec);

        if streams.is_empty() {
            return Err(BiliClientError::ApiError(
                "No streams available".to_string(),
            ));
        }

        // Find the matching protocol
        let target_protocol = match protocol {
            Protocol::HttpStream => "http_stream",
            Protocol::HttpHls => "http_hls",
        };

        let stream = streams
            .iter()
            .find(|s| s["protocol_name"].as_str() == Some(target_protocol))
            .ok_or_else(|| {
                BiliClientError::ApiError(format!("Protocol {} not found", target_protocol))
            })?;

        // Find the matching format
        let target_format = match format {
            Format::Flv => "flv",
            Format::TS => "ts",
            Format::FMP4 => "fmp4",
        };

        let empty_vec = vec![];
        let format_info = stream["format"]
            .as_array()
            .unwrap_or(&empty_vec)
            .iter()
            .find(|f| f["format_name"].as_str() == Some(target_format))
            .ok_or_else(|| BiliClientError::FormatNotFound(target_format.to_owned()))?;

        // Find the matching codec
        let target_codecs = codec
            .iter()
            .map(|c| match c {
                Codec::Avc => "avc",
                Codec::Hevc => "hevc",
            })
            .collect::<Vec<&str>>();

        let codec_info = format_info["codec"]
            .as_array()
            .unwrap_or(&empty_vec)
            .iter()
            .find(|c| target_codecs.contains(&c["codec_name"].as_str().unwrap_or("")))
            .ok_or_else(|| BiliClientError::CodecNotFound(target_codecs.join(",")))?;

        let url_info = codec_info["url_info"].as_array().unwrap_or(&empty_vec);

        let url_info = url_info
            .iter()
            .map(|u| UrlInfo {
                host: u["host"].as_str().unwrap_or("").to_string(),
                extra: u["extra"].as_str().unwrap_or("").to_string(),
            })
            .collect();

        let drm = codec_info["drm"].as_bool().unwrap_or(false);
        let base_url = codec_info["base_url"].as_str().unwrap_or("").to_string();
        let master_url = format_info["master_url"].as_str().map(|s| s.to_string());
        let codec = codec_info["codec_name"].as_str().unwrap_or("");
        let codec = match codec {
            "avc" => Codec::Avc,
            "hevc" => Codec::Hevc,
            _ => return Err(BiliClientError::CodecNotFound(codec.to_string())),
        };

        Ok(BiliStream {
            format,
            codec,
            base_url,
            url_info,
            drm,
            master_url,
        })
    }

    /// Download file from url to path
    pub async fn download_file(&self, url: &str, path: &Path) -> Result<(), BiliClientError> {
        if !path.parent().unwrap().exists() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        let response = self.client.get(url).send().await?;
        let bytes = response.bytes().await?;
        let mut file = tokio::fs::File::create(&path).await?;
        let mut content = std::io::Cursor::new(bytes);
        tokio::io::copy(&mut content, &mut file).await?;
        Ok(())
    }

    pub async fn get_index_content(
        &self,
        account: &AccountRow,
        url: &String,
    ) -> Result<String, BiliClientError> {
        let mut headers = self.generate_user_agent_header();
        if let Ok(cookies) = account.cookies.parse() {
            headers.insert("cookie", cookies);
        } else {
            return Err(BiliClientError::InvalidCookie);
        }
        let response = self
            .client
            .get(url.to_owned())
            .headers(headers)
            .send()
            .await?;

        if response.status().is_success() {
            Ok(response.text().await?)
        } else {
            log::error!("get_index_content failed: {}", response.status());
            Err(BiliClientError::InvalidStream)
        }
    }

    #[allow(unused)]
    pub async fn download_ts(&self, url: &str, file_path: &str) -> Result<u64, BiliClientError> {
        let res = self
            .client
            .get(url)
            .headers(self.generate_user_agent_header())
            .send()
            .await?;
        let mut file = tokio::fs::File::create(file_path).await?;
        let bytes = res.bytes().await?;
        let size = bytes.len() as u64;
        let mut content = std::io::Cursor::new(bytes);
        tokio::io::copy(&mut content, &mut file).await?;
        Ok(size)
    }

    pub async fn download_ts_raw(&self, url: &str) -> Result<Vec<u8>, BiliClientError> {
        let res = self.client.get(url).send().await?;
        let bytes = res.bytes().await?;
        Ok(bytes.to_vec())
    }

    // Method from js code
    pub async fn get_sign(&self, mut parameters: Value) -> Result<String, BiliClientError> {
        let table = vec![
            46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42,
            19, 29, 28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60,
            51, 30, 4, 22, 25, 54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
        ];
        let nav_info: Value = self
            .client
            .get("https://api.bilibili.com/x/web-interface/nav")
            .headers(self.generate_user_agent_header())
            .send()
            .await?
            .json()
            .await?;
        let re = Regex::new(r"wbi/(.*).png").unwrap();
        let img = re
            .captures(nav_info["data"]["wbi_img"]["img_url"].as_str().unwrap())
            .unwrap()
            .get(1)
            .unwrap()
            .as_str();
        let sub = re
            .captures(nav_info["data"]["wbi_img"]["sub_url"].as_str().unwrap())
            .unwrap()
            .get(1)
            .unwrap()
            .as_str();
        let raw_string = format!("{img}{sub}");
        let mut encoded = Vec::new();
        for x in table {
            if x < raw_string.len() {
                encoded.push(raw_string.as_bytes()[x]);
            }
        }
        // only keep 32 bytes of encoded
        encoded = encoded[0..32].to_vec();
        let encoded = String::from_utf8(encoded).unwrap();
        // Timestamp in seconds
        let wts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        parameters
            .as_object_mut()
            .unwrap()
            .insert("wts".to_owned(), serde_json::Value::String(wts.to_string()));
        // Get all keys from parameters into vec
        let mut keys = parameters
            .as_object()
            .unwrap()
            .keys()
            .map(std::borrow::ToOwned::to_owned)
            .collect::<Vec<String>>();
        // sort keys
        keys.sort();
        let mut params = String::new();
        for x in &keys {
            params.push_str(x);
            params.push('=');
            // Value filters !'()* characters
            let value = parameters
                .get(x)
                .unwrap()
                .as_str()
                .unwrap()
                .replace(['!', '\'', '(', ')', '*'], "");
            let value = PctString::encode(value.chars(), URIReserved);
            params.push_str(value.as_str());
            // add & if not last
            if x != keys.last().unwrap() {
                params.push('&');
            }
        }
        // md5 params+encoded
        let w_rid = md5::compute(params.to_string() + encoded.as_str());
        let params = params + format!("&w_rid={w_rid:x}").as_str();
        Ok(params)
    }

    async fn preupload_video(
        &self,
        account: &AccountRow,
        video_file: &Path,
    ) -> Result<PreuploadResponse, BiliClientError> {
        let mut headers = self.generate_user_agent_header();
        if let Ok(cookies) = account.cookies.parse() {
            headers.insert("cookie", cookies);
        } else {
            return Err(BiliClientError::InvalidCookie);
        }
        let url = format!(
            "https://member.bilibili.com/preupload?name={}&r=upos&profile=ugcfx/bup",
            video_file.file_name().unwrap().to_str().unwrap()
        );
        let response = self
            .client
            .get(&url)
            .headers(headers)
            .send()
            .await?
            .json::<PreuploadResponse>()
            .await?;
        Ok(response)
    }

    async fn post_video_meta(
        &self,
        preupload_response: &PreuploadResponse,
        video_file: &Path,
    ) -> Result<PostVideoMetaResponse, BiliClientError> {
        let url = format!(
            "https:{}{}?uploads=&output=json&profile=ugcfx/bup&filesize={}&partsize={}&biz_id={}",
            preupload_response.endpoint,
            preupload_response.upos_uri.replace("upos:/", ""),
            video_file.metadata().unwrap().len(),
            preupload_response.chunk_size,
            preupload_response.biz_id
        );
        let response = self
            .client
            .post(&url)
            .header("X-Upos-Auth", &preupload_response.auth)
            .send()
            .await?
            .json::<PostVideoMetaResponse>()
            .await?;
        Ok(response)
    }

    async fn upload_video(&self, params: UploadParams<'_>) -> Result<usize, BiliClientError> {
        let mut file = File::open(params.video_file).await?;
        let mut buffer = vec![0; params.preupload_response.chunk_size];
        let file_size = params.video_file.metadata()?.len();
        let chunk_size = params.preupload_response.chunk_size as u64;
        let total_chunks = (file_size as f64 / chunk_size as f64).ceil() as usize;

        let start = Instant::now();
        let mut chunk = 0;
        let mut read_total = 0;
        let max_retries = 3;
        let timeout = Duration::from_secs(30);

        while let Ok(size) = file.read(&mut buffer[read_total..]).await {
            // Check for cancellation
            if params
                .reporter
                .cancel
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                return Err(BiliClientError::UploadCancelled);
            }

            read_total += size;
            log::debug!("size: {size}, total: {read_total}");
            if size > 0 && (read_total as u64) < chunk_size {
                continue;
            }
            if size == 0 && read_total == 0 {
                break;
            }

            let mut retry_count = 0;
            let mut success = false;

            while retry_count < max_retries && !success {
                // Check for cancellation before each retry
                if params
                    .reporter
                    .cancel
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    return Err(BiliClientError::UploadCancelled);
                }

                let url = format!(
                    "https:{}{}?partNumber={}&uploadId={}&chunk={}&chunks={}&size={}&start={}&end={}&total={}",
                    params.preupload_response.endpoint,
                    params.preupload_response.upos_uri.replace("upos:/", ""),
                    chunk + 1,
                    params.post_video_meta_response.upload_id,
                    chunk,
                    total_chunks,
                    read_total,
                    chunk * params.preupload_response.chunk_size,
                    chunk * params.preupload_response.chunk_size + read_total,
                    params.video_file.metadata().unwrap().len()
                );

                match self
                    .client
                    .put(&url)
                    .header("X-Upos-Auth", &params.preupload_response.auth)
                    .header("Content-Type", "application/octet-stream")
                    .header("Content-Length", read_total.to_string())
                    .timeout(timeout)
                    .body(buffer[..read_total].to_vec())
                    .send()
                    .await
                {
                    Ok(response) => {
                        if response.status().is_success() {
                            success = true;
                            let _ = response.text().await?;
                        } else {
                            log::error!("Upload failed with status: {}", response.status());
                            retry_count += 1;
                            if retry_count < max_retries {
                                tokio::time::sleep(Duration::from_secs(
                                    2u64.pow(retry_count as u32),
                                ))
                                .await;
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Upload error: {e}");
                        retry_count += 1;
                        if retry_count < max_retries {
                            tokio::time::sleep(Duration::from_secs(2u64.pow(retry_count as u32)))
                                .await;
                        }
                    }
                }
            }

            if !success {
                return Err(BiliClientError::UploadError {
                    err: format!("Failed to upload chunk {chunk} after {max_retries} retries"),
                });
            }

            chunk += 1;
            read_total = 0;
            log::debug!(
                "[bili]speed: {:.1} KiB/s",
                (chunk * params.preupload_response.chunk_size) as f64
                    / start.elapsed().as_secs_f64()
                    / 1024.0
            );

            params.reporter.update(
                format!(
                    "{:.1}% | {:.1} KiB/s",
                    (chunk * 100) as f64 / total_chunks as f64,
                    (chunk * params.preupload_response.chunk_size) as f64
                        / start.elapsed().as_secs_f64()
                        / 1024.0
                )
                .as_str(),
            );
        }
        Ok(total_chunks)
    }

    async fn end_upload(
        &self,
        preupload_response: &PreuploadResponse,
        post_video_meta_response: &PostVideoMetaResponse,
        chunks: usize,
    ) -> Result<(), BiliClientError> {
        let url = format!(
            "https:{}{}?output=json&name={}&profile=ugcfx/bup&uploadId={}&biz_id={}",
            preupload_response.endpoint,
            preupload_response.upos_uri.replace("upos:/", ""),
            preupload_response.upos_uri,
            post_video_meta_response.upload_id,
            preupload_response.biz_id
        );
        let parts: Vec<Value> = (1..=chunks)
            .map(|i| json!({ "partNumber": i, "eTag": "etag" }))
            .collect();
        let body = json!({ "parts": parts });
        self.client
            .post(&url)
            .header("X-Upos-Auth", &preupload_response.auth)
            .header("Content-Type", "application/json; charset=UTF-8")
            .body(body.to_string())
            .send()
            .await?
            .text()
            .await?;
        Ok(())
    }

    pub async fn prepare_video(
        &self,
        reporter: &ProgressReporter,
        account: &AccountRow,
        video_file: &Path,
    ) -> Result<profile::Video, BiliClientError> {
        log::info!("Start Preparing Video: {}", video_file.to_str().unwrap());
        let preupload = self.preupload_video(account, video_file).await?;
        log::info!("Preupload Response: {preupload:?}");
        let metaposted = self.post_video_meta(&preupload, video_file).await?;
        log::info!("Post Video Meta Response: {metaposted:?}");
        let uploaded = self
            .upload_video(UploadParams {
                reporter,
                preupload_response: &preupload,
                post_video_meta_response: &metaposted,
                video_file,
            })
            .await?;
        log::info!("Uploaded: {uploaded}");
        self.end_upload(&preupload, &metaposted, uploaded).await?;
        let filename = Path::new(&metaposted.key)
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap();
        Ok(profile::Video {
            title: filename.to_string(),
            filename: filename.to_string(),
            desc: String::new(),
            cid: preupload.biz_id,
        })
    }

    pub async fn submit_video(
        &self,
        account: &AccountRow,
        profile_template: &Profile,
        video: &profile::Video,
    ) -> Result<VideoSubmitData, BiliClientError> {
        let mut headers = self.generate_user_agent_header();
        if let Ok(cookies) = account.cookies.parse() {
            headers.insert("cookie", cookies);
        } else {
            return Err(BiliClientError::InvalidCookie);
        }
        let url = format!(
            "https://member.bilibili.com/x/vu/web/add/v3?ts={}&csrf={}",
            chrono::Local::now().timestamp(),
            account.csrf
        );
        let mut preprofile = profile_template.clone();
        preprofile.videos.push(video.clone());
        match self
            .client
            .post(&url)
            .headers(headers)
            .header("Content-Type", "application/json; charset=UTF-8")
            .body(serde_json::ser::to_string(&preprofile).unwrap_or_default())
            .send()
            .await
        {
            Ok(raw_resp) => {
                let json: Value = raw_resp.json().await?;
                if let Ok(resp) = serde_json::from_value::<GeneralResponse>(json.clone()) {
                    match resp.data {
                        response::Data::VideoSubmit(data) => Ok(data),
                        _ => Err(BiliClientError::InvalidResponse),
                    }
                } else {
                    log::error!("Parse response failed: {json}");
                    Err(BiliClientError::InvalidResponse)
                }
            }
            Err(e) => {
                log::error!("Send failed {e}");
                Err(BiliClientError::InvalidResponse)
            }
        }
    }

    pub async fn upload_cover(
        &self,
        account: &AccountRow,
        cover: &str,
    ) -> Result<String, BiliClientError> {
        let url = format!(
            "https://member.bilibili.com/x/vu/web/cover/up?ts={}",
            chrono::Local::now().timestamp(),
        );
        let mut headers = self.generate_user_agent_header();
        if let Ok(cookies) = account.cookies.parse() {
            headers.insert("cookie", cookies);
        } else {
            return Err(BiliClientError::InvalidCookie);
        }
        let params = [("csrf", account.csrf.clone()), ("cover", cover.to_string())];
        match self
            .client
            .post(&url)
            .headers(headers)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&params)
            .send()
            .await
        {
            Ok(raw_resp) => {
                let json: Value = raw_resp.json().await?;
                if let Ok(resp) = serde_json::from_value::<GeneralResponse>(json.clone()) {
                    match resp.data {
                        response::Data::Cover(data) => Ok(data.url),
                        _ => Err(BiliClientError::InvalidResponse),
                    }
                } else {
                    log::error!("Parse response failed: {json}");
                    Err(BiliClientError::InvalidResponse)
                }
            }
            Err(e) => {
                log::error!("Send failed {e}");
                Err(BiliClientError::InvalidResponse)
            }
        }
    }

    pub async fn send_danmaku(
        &self,
        account: &AccountRow,
        room_id: i64,
        message: &str,
    ) -> Result<(), BiliClientError> {
        let url = "https://api.live.bilibili.com/msg/send".to_string();
        let mut headers = self.generate_user_agent_header();
        if let Ok(cookies) = account.cookies.parse() {
            headers.insert("cookie", cookies);
        } else {
            return Err(BiliClientError::InvalidCookie);
        }
        let params = [
            ("bubble", "0"),
            ("msg", message),
            ("color", "16777215"),
            ("mode", "1"),
            ("fontsize", "25"),
            ("room_type", "0"),
            ("rnd", &format!("{}", chrono::Local::now().timestamp())),
            ("roomid", &format!("{room_id}")),
            ("csrf", &account.csrf),
            ("csrf_token", &account.csrf),
        ];
        let _ = self
            .client
            .post(&url)
            .headers(headers)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&params)
            .send()
            .await?;
        Ok(())
    }

    pub async fn get_video_typelist(
        &self,
        account: &AccountRow,
    ) -> Result<Vec<response::Typelist>, BiliClientError> {
        let url = "https://member.bilibili.com/x/vupre/web/archive/pre?lang=cn";
        let mut headers = self.generate_user_agent_header();
        if let Ok(cookies) = account.cookies.parse() {
            headers.insert("cookie", cookies);
        } else {
            return Err(BiliClientError::InvalidCookie);
        }
        let resp: GeneralResponse = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await?
            .json()
            .await?;
        if resp.code == 0 {
            if let response::Data::VideoTypeList(data) = resp.data {
                Ok(data.typelist)
            } else {
                Err(BiliClientError::InvalidResponse)
            }
        } else {
            log::error!("Get video typelist failed with code {}", resp.code);
            Err(BiliClientError::InvalidResponse)
        }
    }
}
