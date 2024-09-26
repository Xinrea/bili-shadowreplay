pub mod errors;
pub mod profile;
pub mod response;
use crate::db::AccountRow;

use super::StreamType;
use errors::BiliClientError;
use pct_str::PctString;
use pct_str::URIReserved;
use profile::Profile;
use regex::Regex;
use reqwest::Client;
use response::GeneralResponse;
use response::PostVideoMetaResponse;
use response::PreuploadResponse;
use response::VideoSubmitData;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use serde_json::Value;
use std::path::Path;
use std::time::SystemTime;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::sync::RwLock;
use tokio::time::Instant;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayUrlResponse {
    pub code: i64,
    pub message: String,
    pub ttl: i64,
    pub data: Data,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Data {
    #[serde(rename = "room_id")]
    pub room_id: i64,
    #[serde(rename = "short_id")]
    pub short_id: i64,
    pub uid: i64,
    #[serde(rename = "is_hidden")]
    pub is_hidden: bool,
    #[serde(rename = "is_locked")]
    pub is_locked: bool,
    #[serde(rename = "is_portrait")]
    pub is_portrait: bool,
    #[serde(rename = "live_status")]
    pub live_status: i64,
    #[serde(rename = "hidden_till")]
    pub hidden_till: i64,
    #[serde(rename = "lock_till")]
    pub lock_till: i64,
    pub encrypted: bool,
    #[serde(rename = "pwd_verified")]
    pub pwd_verified: bool,
    #[serde(rename = "live_time")]
    pub live_time: i64,
    #[serde(rename = "room_shield")]
    pub room_shield: i64,
    #[serde(rename = "all_special_types")]
    pub all_special_types: Vec<i64>,
    #[serde(rename = "playurl_info")]
    pub playurl_info: PlayurlInfo,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayurlInfo {
    #[serde(rename = "conf_json")]
    pub conf_json: String,
    pub playurl: Playurl,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playurl {
    pub cid: i64,
    #[serde(rename = "g_qn_desc")]
    pub g_qn_desc: Vec<GQnDesc>,
    pub stream: Vec<Stream>,
    #[serde(rename = "p2p_data")]
    pub p2p_data: P2pData,
    #[serde(rename = "dolby_qn")]
    pub dolby_qn: Value,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GQnDesc {
    pub qn: i64,
    pub desc: String,
    #[serde(rename = "hdr_desc")]
    pub hdr_desc: String,
    #[serde(rename = "attr_desc")]
    pub attr_desc: Value,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stream {
    #[serde(rename = "protocol_name")]
    pub protocol_name: String,
    pub format: Vec<Format>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Format {
    #[serde(rename = "format_name")]
    pub format_name: String,
    pub codec: Vec<Codec>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Codec {
    #[serde(rename = "codec_name")]
    pub codec_name: String,
    #[serde(rename = "current_qn")]
    pub current_qn: i64,
    #[serde(rename = "accept_qn")]
    pub accept_qn: Vec<i64>,
    #[serde(rename = "base_url")]
    pub base_url: String,
    #[serde(rename = "url_info")]
    pub url_info: Vec<UrlInfo>,
    #[serde(rename = "hdr_qn")]
    pub hdr_qn: Value,
    #[serde(rename = "dolby_type")]
    pub dolby_type: i64,
    #[serde(rename = "attr_name")]
    pub attr_name: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlInfo {
    pub host: String,
    pub extra: String,
    #[serde(rename = "stream_ttl")]
    pub stream_ttl: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct P2pData {
    pub p2p: bool,
    #[serde(rename = "p2p_type")]
    pub p2p_type: i64,
    #[serde(rename = "m_p2p")]
    pub m_p2p: bool,
    #[serde(rename = "m_servers")]
    pub m_servers: Value,
}

/// BiliClient is thread safe
pub struct BiliClient {
    client: Client,
    headers: reqwest::header::HeaderMap,
    extra: RwLock<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RoomInfo {
    pub live_status: u8,
    pub room_cover_url: String,
    pub room_id: u64,
    pub room_keyframe_url: String,
    pub room_title: String,
    pub user_id: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UserInfo {
    pub user_id: u64,
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

impl BiliClient {
    pub fn new() -> Result<BiliClient, BiliClientError> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("authority", "api.live.bilibili.com".parse().unwrap());
        headers.insert("accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7".parse().unwrap());
        headers.insert(
            "accept-language",
            "zh-CN,zh;q=0.9,en;q=0.8".parse().unwrap(),
        );
        headers.insert("cache-control", "max-age=0".parse().unwrap());
        headers.insert(
            "sec-ch-ua",
            "\"Google Chrome\";v=\"111\", \"Not(A:Brand\";v=\"8\", \"Chromium\";v=\"111\""
                .parse()
                .unwrap(),
        );
        headers.insert("sec-ch-ua-mobile", "?0".parse().unwrap());
        headers.insert("sec-ch-ua-platform", "\"macOS\"".parse().unwrap());
        headers.insert("sec-fetch-dest", "document".parse().unwrap());
        headers.insert("sec-fetch-mode", "navigate".parse().unwrap());
        headers.insert("sec-fetch-site", "none".parse().unwrap());
        headers.insert("sec-fetch-user", "?1".parse().unwrap());
        headers.insert("upgrade-insecure-requests", "1".parse().unwrap());
        headers.insert("user-agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/111.0.0.0 Safari/537.36".parse().unwrap());

        if let Ok(client) = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
        {
            Ok(BiliClient {
                client,
                headers,
                extra: RwLock::new("".into()),
            })
        } else {
            Err(BiliClientError::InitClientError)
        }
    }

    pub async fn get_qr(&self) -> Result<QrInfo, BiliClientError> {
        let res: serde_json::Value = self
            .client
            .get("https://passport.bilibili.com/x/passport-login/web/qrcode/generate")
            .headers(self.headers.clone())
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
        let res: serde_json::Value = self
            .client
            .get(format!(
                "https://passport.bilibili.com/x/passport-login/web/qrcode/poll?qrcode_key={}",
                qrcode_key
            ))
            .headers(self.headers.clone())
            .send()
            .await?
            .json()
            .await?;
        let code: u8 = res["data"]["code"].as_u64().unwrap_or(400) as u8;
        let mut cookies: String = "".to_string();
        if code == 0 {
            let url = res["data"]["url"]
                .as_str()
                .ok_or(BiliClientError::InvalidValue)?
                .to_string();
            let query_str = url.split('?').last().unwrap();
            cookies = query_str.replace('&', ";");
        }
        Ok(QrStatus { code, cookies })
    }

    pub async fn logout(&self, account: &AccountRow) -> Result<(), BiliClientError> {
        let url = "https://passport.bilibili.com/login/exit/v2";
        let mut headers = self.headers.clone();
        headers.insert("cookie", account.cookies.parse().unwrap());
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
        user_id: u64,
    ) -> Result<UserInfo, BiliClientError> {
        let params: Value = json!({
            "mid": user_id.to_string(),
            "platform": "web",
            "web_location": "1550101",
            "token": ""
        });
        let params = self.get_sign(params).await?;
        let mut headers = self.headers.clone();
        headers.insert("cookie", account.cookies.parse().unwrap());
        let res: serde_json::Value = self
            .client
            .get(format!(
                "https://api.bilibili.com/x/space/wbi/acc/info?{}",
                params
            ))
            .headers(headers)
            .send()
            .await?
            .json()
            .await?;
        Ok(UserInfo {
            user_id: user_id,
            user_name: res["data"]["name"].as_str().unwrap_or("").to_string(),
            user_sign: res["data"]["sign"].as_str().unwrap_or("").to_string(),
            user_avatar_url: res["data"]["face"].as_str().unwrap_or("").to_string(),
        })
    }

    pub async fn get_room_info(
        &self,
        account: &AccountRow,
        room_id: u64,
    ) -> Result<RoomInfo, BiliClientError> {
        let mut headers = self.headers.clone();
        headers.insert("cookie", account.cookies.parse().unwrap());
        let res: serde_json::Value = self
            .client
            .get(format!(
                "https://api.live.bilibili.com/room/v1/Room/get_info?room_id={}",
                room_id
            ))
            .headers(headers)
            .send()
            .await?
            .json()
            .await?;
        let code = res["code"].as_u64().ok_or(BiliClientError::InvalidValue)?;
        if code != 0 {
            return Err(BiliClientError::InvalidCode);
        }

        let room_id = res["data"]["room_id"]
            .as_u64()
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
            .as_u64()
            .ok_or(BiliClientError::InvalidValue)?;
        let live_status = res["data"]["live_status"]
            .as_u64()
            .ok_or(BiliClientError::InvalidValue)? as u8;
        Ok(RoomInfo {
            room_id,
            room_title,
            room_cover_url,
            room_keyframe_url,
            user_id,
            live_status,
        })
    }

    pub async fn get_play_url(
        &self,
        account: &AccountRow,
        room_id: u64,
    ) -> Result<(String, StreamType), BiliClientError> {
        let mut headers = self.headers.clone();
        headers.insert("cookie", account.cookies.parse().unwrap());
        let res: PlayUrlResponse = self
            .client
            .get(format!(
                "https://api.live.bilibili.com/xlive/web-room/v2/index/getRoomPlayInfo?room_id={}&protocol=1&format=0,1,2&codec=0&qn=10000&platform=h5",
                room_id
            ))
            .headers(headers)
            .send().await?
            .json().await?;
        if res.code == 0 {
            if let Some(stream) = res.data.playurl_info.playurl.stream.first() {
                // Get fmp4 format
                if let Some(format) = stream.format.get(1) {
                    self.get_url_from_format(format)
                        .await
                        .ok_or(BiliClientError::InvalidFormat)
                        .map(|url| (url, StreamType::FMP4))
                } else if let Some(format) = stream.format.first() {
                    self.get_url_from_format(format)
                        .await
                        .ok_or(BiliClientError::InvalidFormat)
                        .map(|url| (url, StreamType::TS))
                } else {
                    Err(BiliClientError::InvalidResponse)
                }
            } else {
                Err(BiliClientError::InvalidResponse)
            }
        } else {
            Err(BiliClientError::InvalidResponse)
        }
    }

    async fn get_url_from_format(&self, format: &Format) -> Option<String> {
        if let Some(codec) = format.codec.first() {
            if let Some(url_info) = codec.url_info.first() {
                let base_url = codec.base_url.strip_suffix('?').unwrap();
                let extra = "?".to_owned() + &url_info.extra.clone();
                let host = url_info.host.clone();
                let url = format!("{}{}", host, base_url);
                *self.extra.write().await = extra;
                Some(url)
            } else {
                None
            }
        } else {
            None
        }
    }

    pub async fn get_index_content(&self, url: &String) -> Result<String, BiliClientError> {
        Ok(self
            .client
            .get(url.to_owned() + self.extra.read().await.as_str())
            .headers(self.headers.clone())
            .send()
            .await?
            .text()
            .await?)
    }

    pub async fn download_ts(&self, url: &str, file_path: &str) -> Result<(), BiliClientError> {
        let url = url.to_owned() + self.extra.read().await.as_str();
        let res = self
            .client
            .get(url)
            .headers(self.headers.clone())
            .send()
            .await?;
        if let Ok(mut file) = std::fs::File::create(file_path) {
            let mut content = std::io::Cursor::new(res.bytes().await?);
            std::io::copy(&mut content, &mut file).unwrap();
        } else {
            log::error!("Failed to create file {}", file_path);
        }
        Ok(())
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
            .headers(self.headers.clone())
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
        let raw_string = format!("{}{}", img, sub);
        let mut encoded = Vec::new();
        table.into_iter().for_each(|x| {
            if x < raw_string.len() {
                encoded.push(raw_string.as_bytes()[x]);
            }
        });
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
            .map(|x| x.to_owned())
            .collect::<Vec<String>>();
        // sort keys
        keys.sort();
        let mut params = String::new();
        keys.iter().for_each(|x| {
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
        });
        // md5 params+encoded
        let w_rid = md5::compute(params.to_string() + encoded.as_str());
        let params = params + format!("&w_rid={:x}", w_rid).as_str();
        Ok(params)
    }

    async fn preupload_video(
        &self,
        account: &AccountRow,
        video_file: &Path,
    ) -> Result<PreuploadResponse, BiliClientError> {
        let mut headers = self.headers.clone();
        headers.insert("cookie", account.cookies.parse().unwrap());
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

    async fn upload_video(
        &self,
        preupload_response: &PreuploadResponse,
        post_video_meta_response: &PostVideoMetaResponse,
        video_file: &Path,
    ) -> Result<usize, BiliClientError> {
        let mut file = File::open(video_file).await?;
        let mut buffer = vec![0; preupload_response.chunk_size];
        let file_size = video_file.metadata()?.len();
        let chunk_size = preupload_response.chunk_size as u64; // 确保使用 u64 类型
        let total_chunks = (file_size as f64 / chunk_size as f64).ceil() as usize; // 计算总分块数

        let start = Instant::now();
        let mut chunk = 0;
        let mut read_total = 0;
        while let Ok(size) = file.read(&mut buffer[read_total..]).await {
            read_total += size;
            log::debug!("size: {}, total: {}", size, read_total);
            if size > 0 && (read_total as u64) < chunk_size {
                continue;
            }
            if size == 0 && read_total == 0 {
                break;
            }
            let url = format!(
                "https:{}{}?partNumber={}&uploadId={}&chunk={}&chunks={}&size={}&start={}&end={}&total={}",
                preupload_response.endpoint,
                preupload_response.upos_uri.replace("upos:/", ""),
                chunk + 1,
                post_video_meta_response.upload_id,
                chunk,
                total_chunks,
                read_total,
                chunk * preupload_response.chunk_size,
                chunk * preupload_response.chunk_size + read_total,
                video_file.metadata().unwrap().len()
            );
            self.client
                .put(&url)
                .header("X-Upos-Auth", &preupload_response.auth)
                .header("Content-Type", "application/octet-stream")
                .header("Content-Length", read_total.to_string())
                .body(buffer[..read_total].to_vec())
                .send()
                .await?
                .text()
                .await?;
            chunk += 1;
            read_total = 0;
            log::debug!(
                "[bili]speed: {:.1} KiB/s",
                (chunk * preupload_response.chunk_size) as f64
                    / start.elapsed().as_secs_f64()
                    / 1024.0
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
        account: &AccountRow,
        video_file: &Path,
    ) -> Result<profile::Video, BiliClientError> {
        let preupload = self.preupload_video(account, video_file).await?;
        let metaposted = self.post_video_meta(&preupload, video_file).await?;
        let uploaded = self
            .upload_video(&preupload, &metaposted, video_file)
            .await?;
        self.end_upload(&preupload, &metaposted, uploaded).await?;
        let filename = Path::new(&metaposted.key)
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap();
        Ok(profile::Video {
            title: "".to_string(),
            filename: filename.to_string(),
            desc: "".to_string(),
            cid: preupload.biz_id,
        })
    }

    pub async fn submit_video(
        &self,
        account: &AccountRow,
        profile_template: &Profile,
        video: &profile::Video,
    ) -> Result<VideoSubmitData, BiliClientError> {
        let mut headers = self.headers.clone();
        headers.insert("cookie", account.cookies.parse().unwrap());
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
            .body(serde_json::ser::to_string(&preprofile).unwrap_or("".to_string()))
            .send()
            .await
        {
            Ok(raw_resp) => {
                let json = raw_resp.json().await?;
                if let Ok(resp) = serde_json::from_value::<GeneralResponse>(json) {
                    match resp.data {
                        response::Data::VideoSubmit(data) => Ok(data),
                        _ => Err(BiliClientError::InvalidResponse),
                    }
                } else {
                    println!("Parse response failed");
                    Err(BiliClientError::InvalidResponse)
                }
            }
            Err(e) => {
                println!("Send failed {}", e);
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
        let mut headers = self.headers.clone();
        headers.insert("cookie", account.cookies.parse().unwrap());
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
                let json = raw_resp.json().await?;
                if let Ok(resp) = serde_json::from_value::<GeneralResponse>(json) {
                    match resp.data {
                        response::Data::Cover(data) => Ok(data.url),
                        _ => Err(BiliClientError::InvalidResponse),
                    }
                } else {
                    println!("Parse response failed");
                    Err(BiliClientError::InvalidResponse)
                }
            }
            Err(e) => {
                println!("Send failed {}", e);
                Err(BiliClientError::InvalidResponse)
            }
        }
    }
}
