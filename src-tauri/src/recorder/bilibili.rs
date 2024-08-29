pub mod errors;
use errors::BiliClientError;
use pct_str::PctString;
use pct_str::URIReserved;
use regex::Regex;
use reqwest::blocking::Client;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use serde_json::Value;
use std::sync::Mutex;
use std::time::SystemTime;

use super::StreamType;

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

pub struct BiliClient {
    client: Client,
    headers: reqwest::header::HeaderMap,
    extra: Mutex<String>,
}

#[derive(Debug)]
pub struct RoomInfo {
    pub room_id: u64,
    pub room_title: String,
    pub room_cover_url: String,
    pub room_keyframe_url: String,
    pub user_id: u64,
    pub live_status: u8,
}

#[derive(Debug)]
pub struct UserInfo {
    pub user_id: u64,
    pub user_name: String,
    pub user_sign: String,
    pub user_avatar_url: String,
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
                extra: Mutex::new("".into()),
            })
        } else {
            Err(BiliClientError::InitClientError)
        }
    }

    pub fn get_user_info(&self, user_id: u64) -> Result<UserInfo, BiliClientError> {
        let params: Value = json!({
            "mid": user_id.to_string(),
            "platform": "web",
            "web_location": "1550101",
            "token": ""
        });
        let params = self.get_sign(params)?;
        let res: serde_json::Value = self
            .client
            .get(format!(
                "https://api.bilibili.com/x/space/wbi/acc/info?{}",
                params
            ))
            .headers(self.headers.clone())
            .send()?
            .json()?;
        Ok(UserInfo {
            user_id,
            user_name: res["data"]["name"]
                .as_str()
                .ok_or(BiliClientError::InvalidValue)?
                .to_string(),
            user_sign: res["data"]["sign"]
                .as_str()
                .ok_or(BiliClientError::InvalidValue)?
                .to_string(),
            user_avatar_url: res["data"]["face"]
                .as_str()
                .ok_or(BiliClientError::InvalidValue)?
                .to_string(),
        })
    }

    pub fn get_room_info(&self, room_id: u64) -> Result<RoomInfo, BiliClientError> {
        let res: serde_json::Value = self
            .client
            .get(format!(
                "https://api.live.bilibili.com/room/v1/Room/get_info?room_id={}",
                room_id
            ))
            .headers(self.headers.clone())
            .send()?
            .json()?;
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

    pub fn get_play_url(&self, room_id: u64) -> Result<(String, StreamType), BiliClientError> {
        let res: PlayUrlResponse = self
            .client
            .get(format!(
                "https://api.live.bilibili.com/xlive/web-room/v2/index/getRoomPlayInfo?room_id={}&protocol=1&format=0,1,2&codec=0&qn=10000&platform=h5",
                room_id
            ))
            .headers(self.headers.clone())
            .send()?
            .json()?;
        if res.code == 0 {
            if let Some(stream) = res.data.playurl_info.playurl.stream.get(0) {
                // Get fmp4 format
                if let Some(format) = stream.format.get(1) {
                    self.get_url_from_format(format)
                        .ok_or(BiliClientError::InvalidFormat)
                        .map(|url| (url, StreamType::FMP4))
                } else if let Some(format) = stream.format.get(0) {
                    self.get_url_from_format(format)
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

    fn get_url_from_format(&self, format: &Format) -> Option<String> {
        if let Some(codec) = format.codec.get(0) {
            if let Some(url_info) = codec.url_info.get(0) {
                let base_url = codec.base_url.strip_suffix('?').unwrap();
                let extra = "?".to_owned() + &url_info.extra.clone();
                let host = url_info.host.clone();
                let url = format!("{}{}", host, base_url);
                *self.extra.lock().unwrap() = extra;
                Some(url)
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn get_index_content(&self, url: &String) -> Result<String, BiliClientError> {
        Ok(self
            .client
            .get(url.to_owned() + self.extra.lock().unwrap().as_str())
            .headers(self.headers.clone())
            .send()?
            .text()?)
    }

    pub fn download_ts(
        &self,
        cache_path: &str,
        room_id: u64,
        url: &str,
    ) -> Result<(), BiliClientError> {
        let (tmp_path, file_name) = Self::url_to_file_name(cache_path, room_id, url);
        std::fs::create_dir_all(tmp_path).expect("create tmp_path failed");
        let url = url.to_owned() + self.extra.lock().unwrap().as_str();
        let res = self.client.get(url).headers(self.headers.clone()).send()?;
        let mut file = std::fs::File::create(file_name).unwrap();
        let mut content = std::io::Cursor::new(res.bytes()?);
        std::io::copy(&mut content, &mut file).unwrap();
        Ok(())
    }

    pub fn url_to_file_name(cache_path: &str, room_id: u64, url: &str) -> (String, String) {
        let tmp_path = format!("{}/{}/", cache_path, room_id);
        let url = reqwest::Url::parse(url).unwrap();
        let file_name = url.path_segments().and_then(|x| x.last()).unwrap();
        let full_file = tmp_path.clone() + file_name.split('?').collect::<Vec<&str>>()[0];
        (tmp_path, full_file)
    }

    // Method from js code
    pub fn get_sign(&self, mut parameters: Value) -> Result<String, BiliClientError> {
        let table = vec![
            46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9, 42,
            19, 29, 28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0, 1, 60,
            51, 30, 4, 22, 25, 54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52,
        ];
        let nav_info: Value = self
            .client
            .get("https://api.bilibili.com/x/web-interface/nav")
            .headers(self.headers.clone())
            .send()?
            .json()?;
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
}
