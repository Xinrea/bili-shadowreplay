use super::errors::BiliClientError;
use super::profile;
use super::profile::Profile;
use super::response;
use super::response::GeneralResponse;
use super::response::PostVideoMetaResponse;
use super::response::PreuploadResponse;
use super::response::VideoSubmitData;
use crate::database::account::AccountRow;
use crate::progress_reporter::ProgressReporter;
use crate::progress_reporter::ProgressReporterTrait;
use base64::Engine;
use pct_str::PctString;
use pct_str::URIReserved;
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

/// BiliClient is thread safe
pub struct BiliClient {
    client: Client,
    headers: reqwest::header::HeaderMap,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StreamType {
    TS,
    FMP4,
}

#[derive(Clone, Debug)]
pub struct BiliStream {
    pub format: StreamType,
    pub host: String,
    pub path: String,
    pub extra: String,
    pub expire: i64,
}

impl fmt::Display for BiliStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "type: {:?}, host: {}, path: {}, extra: {}, expire: {}",
            self.format, self.host, self.path, self.extra, self.expire
        )
    }
}

impl BiliStream {
    pub fn new(format: StreamType, base_url: &str, host: &str, extra: &str) -> BiliStream {
        BiliStream {
            format,
            host: host.into(),
            path: BiliStream::get_path(base_url),
            extra: extra.into(),
            expire: BiliStream::get_expire(extra).unwrap_or(600000),
        }
    }

    pub fn index(&self) -> String {
        format!(
            "https://{}/{}/{}?{}",
            self.host, self.path, "index.m3u8", self.extra
        )
    }

    pub fn ts_url(&self, seg_name: &str) -> String {
        format!(
            "https://{}/{}/{}?{}",
            self.host, self.path, seg_name, self.extra
        )
    }

    pub fn get_path(base_url: &str) -> String {
        match base_url.rfind('/') {
            Some(pos) => base_url[..pos + 1].to_string(),
            None => base_url.to_string(),
        }
    }

    pub fn get_expire(extra: &str) -> Option<i64> {
        extra.split('&').find_map(|param| {
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
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("user-agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/111.0.0.0 Safari/537.36".parse().unwrap());

        if let Ok(client) = Client::builder().timeout(Duration::from_secs(10)).build() {
            Ok(BiliClient { client, headers })
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
            let query_str = url.split('?').next_back().unwrap();
            cookies = query_str.replace('&', ";");
        }
        Ok(QrStatus { code, cookies })
    }

    pub async fn logout(&self, account: &AccountRow) -> Result<(), BiliClientError> {
        let url = "https://passport.bilibili.com/login/exit/v2";
        let mut headers = self.headers.clone();
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
        user_id: u64,
    ) -> Result<UserInfo, BiliClientError> {
        let params: Value = json!({
            "mid": user_id.to_string(),
            "platform": "web",
            "web_location": "1550101",
            "token": "",
            "w_webid": "",
        });
        let params = self.get_sign(params).await?;
        let mut headers = self.headers.clone();
        if let Ok(cookies) = account.cookies.parse() {
            headers.insert("cookie", cookies);
        } else {
            return Err(BiliClientError::InvalidCookie);
        }
        let resp = self
            .client
            .get(format!(
                "https://api.bilibili.com/x/space/wbi/acc/info?{}",
                params
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
            log::error!("Get user info failed {}", code);
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
        room_id: u64,
    ) -> Result<RoomInfo, BiliClientError> {
        let mut headers = self.headers.clone();
        if let Ok(cookies) = account.cookies.parse() {
            headers.insert("cookie", cookies);
        } else {
            return Err(BiliClientError::InvalidCookie);
        }
        let response = self
            .client
            .get(format!(
                "https://api.live.bilibili.com/room/v1/Room/get_info?room_id={}",
                room_id
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

    /// Get and encode response data into base64
    pub async fn get_cover_base64(&self, url: &str) -> Result<String, BiliClientError> {
        let response = self.client.get(url).send().await?;
        let bytes = response.bytes().await?;
        let base64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        let mime_type = mime_guess::from_path(url)
            .first_or_octet_stream()
            .to_string();
        Ok(format!("data:{};base64,{}", mime_type, base64))
    }

    pub async fn get_index_content(
        &self,
        account: &AccountRow,
        url: &String,
    ) -> Result<String, BiliClientError> {
        let mut headers = self.headers.clone();
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

    pub async fn download_ts(&self, url: &str, file_path: &str) -> Result<u64, BiliClientError> {
        let res = self
            .client
            .get(url)
            .headers(self.headers.clone())
            .send()
            .await?;
        let mut file = tokio::fs::File::create(file_path).await?;
        let bytes = res.bytes().await?;
        let size = bytes.len() as u64;
        let mut content = std::io::Cursor::new(bytes);
        tokio::io::copy(&mut content, &mut file).await?;
        Ok(size)
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
            log::debug!("size: {}, total: {}", size, read_total);
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
                        log::error!("Upload error: {}", e);
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
                    err: format!(
                        "Failed to upload chunk {} after {} retries",
                        chunk, max_retries
                    ),
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
        log::info!("Preupload Response: {:?}", preupload);
        let metaposted = self.post_video_meta(&preupload, video_file).await?;
        log::info!("Post Video Meta Response: {:?}", metaposted);
        let uploaded = self
            .upload_video(UploadParams {
                reporter,
                preupload_response: &preupload,
                post_video_meta_response: &metaposted,
                video_file,
            })
            .await?;
        log::info!("Uploaded: {}", uploaded);
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
            .body(serde_json::ser::to_string(&preprofile).unwrap_or("".to_string()))
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
                    log::error!("Parse response failed: {}", json);
                    Err(BiliClientError::InvalidResponse)
                }
            }
            Err(e) => {
                log::error!("Send failed {}", e);
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
                    log::error!("Parse response failed: {}", json);
                    Err(BiliClientError::InvalidResponse)
                }
            }
            Err(e) => {
                log::error!("Send failed {}", e);
                Err(BiliClientError::InvalidResponse)
            }
        }
    }

    pub async fn send_danmaku(
        &self,
        account: &AccountRow,
        room_id: u64,
        message: &str,
    ) -> Result<(), BiliClientError> {
        let url = "https://api.live.bilibili.com/msg/send".to_string();
        let mut headers = self.headers.clone();
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
            ("roomid", &format!("{}", room_id)),
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
        let mut headers = self.headers.clone();
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
