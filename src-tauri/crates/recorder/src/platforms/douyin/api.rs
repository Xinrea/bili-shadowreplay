use crate::account::Account;
use crate::errors::RecorderError;
use crate::utils::user_agent_generator;
use deno_core::JsRuntime;
use deno_core::RuntimeOptions;
use reqwest::Client;
use uuid::Uuid;

use super::response::DouyinRoomInfoResponse;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct DouyinBasicRoomInfo {
    pub room_id_str: String,
    pub room_title: String,
    pub cover: Option<String>,
    pub status: i64,
    pub hls_url: String,
    pub stream_data: String,
    // user related
    pub user_name: String,
    pub user_avatar: String,
    pub sec_user_id: String,
}

fn setup_js_runtime() -> Result<JsRuntime, RecorderError> {
    // Create a new V8 runtime
    let mut runtime = JsRuntime::new(RuntimeOptions::default());

    // Add global CryptoJS object
    let crypto_js = include_str!("js/a_bogus.js");
    runtime
        .execute_script(
            "<a_bogus.js>",
            deno_core::FastString::from_static(crypto_js),
        )
        .map_err(|e| RecorderError::JsRuntimeError(format!("Failed to execute crypto-js: {e}")))?;
    Ok(runtime)
}

async fn generate_a_bogus(params: &str, user_agent: &str) -> Result<String, RecorderError> {
    let mut runtime = setup_js_runtime()?;
    // Call the get_wss_url function
    let sign_call = format!("generate_a_bogus(\"{params}\", \"{user_agent}\")");
    let result = runtime
        .execute_script("<sign_call>", deno_core::FastString::from(sign_call))
        .map_err(|e| RecorderError::JsRuntimeError(format!("Failed to execute JavaScript: {e}")))?;

    // Get the result from the V8 runtime
    let mut scope = runtime.handle_scope();
    let local = deno_core::v8::Local::new(&mut scope, result);
    let url = local
        .to_string(&mut scope)
        .unwrap()
        .to_rust_string_lossy(&mut scope);
    Ok(url)
}

async fn generate_ms_token() -> String {
    // generate a random 32 characters uuid string
    let uuid = Uuid::new_v4();
    uuid.to_string()
}

pub fn generate_user_agent_header() -> reqwest::header::HeaderMap {
    let user_agent = user_agent_generator::UserAgentGenerator::new().generate(false);
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("user-agent", user_agent.parse().unwrap());
    headers
}

pub async fn get_room_info(
    client: &Client,
    account: &Account,
    room_id: i64,
    sec_user_id: &str,
) -> Result<DouyinBasicRoomInfo, RecorderError> {
    let mut headers = generate_user_agent_header();
    headers.insert("Referer", "https://live.douyin.com/".parse().unwrap());
    headers.insert("Cookie", account.cookies.clone().parse().unwrap());
    let ms_token = generate_ms_token().await;
    let user_agent = headers.get("user-agent").unwrap().to_str().unwrap();
    let params = format!(
            "aid=6383&app_name=douyin_web&live_id=1&device_platform=web&language=zh-CN&enter_from=web_live&cookie_enabled=true&screen_width=1920&screen_height=1080&browser_language=zh-CN&browser_platform=MacIntel&browser_name=Chrome&browser_version=122.0.0.0&web_rid={room_id}&ms_token={ms_token}");
    let a_bogus = generate_a_bogus(&params, user_agent).await?;
    // log::debug!("params: {params}");
    // log::debug!("user_agent: {user_agent}");
    // log::debug!("a_bogus: {a_bogus}");
    let url = format!(
            "https://live.douyin.com/webcast/room/web/enter/?aid=6383&app_name=douyin_web&live_id=1&device_platform=web&language=zh-CN&enter_from=web_live&cookie_enabled=true&screen_width=1920&screen_height=1080&browser_language=zh-CN&browser_platform=MacIntel&browser_name=Chrome&browser_version=122.0.0.0&web_rid={room_id}&ms_token={ms_token}&a_bogus={a_bogus}"
        );

    let resp = client.get(&url).headers(headers).send().await?;

    let status = resp.status();
    let text = resp.text().await?;

    if text.is_empty() {
        log::debug!("Empty room info response, trying H5 API");
        return get_room_info_h5(client, account, room_id, sec_user_id).await;
    }

    if status.is_success() {
        if let Ok(data) = serde_json::from_str::<DouyinRoomInfoResponse>(&text) {
            let cover = data
                .data
                .data
                .first()
                .and_then(|data| data.cover.as_ref())
                .map(|cover| cover.url_list[0].clone());
            return Ok(DouyinBasicRoomInfo {
                room_id_str: data.data.data[0].id_str.clone(),
                sec_user_id: sec_user_id.to_string(),
                cover,
                room_title: data.data.data[0].title.clone(),
                user_name: data.data.user.nickname.clone(),
                user_avatar: data.data.user.avatar_thumb.url_list[0].clone(),
                status: data.data.room_status,
                hls_url: data.data.data[0]
                    .stream_url
                    .as_ref()
                    .map(|stream_url| stream_url.hls_pull_url.clone())
                    .unwrap_or_default(),
                stream_data: data.data.data[0]
                    .stream_url
                    .as_ref()
                    .map(|s| s.live_core_sdk_data.pull_data.stream_data.clone())
                    .unwrap_or_default(),
            });
        }
        log::error!("Failed to parse room info response: {text}");
        return get_room_info_h5(client, account, room_id, sec_user_id).await;
    }

    log::error!("Failed to get room info: {status}");
    return get_room_info_h5(client, account, room_id, sec_user_id).await;
}

pub async fn get_room_info_h5(
    client: &Client,
    account: &Account,
    room_id: i64,
    sec_user_id: &str,
) -> Result<DouyinBasicRoomInfo, RecorderError> {
    // 参考biliup实现，构建完整的URL参数
    let room_id_str = room_id.to_string();
    // https://webcast.amemv.com/webcast/room/reflow/info/?type_id=0&live_id=1&version_code=99.99.99&app_id=1128&room_id=10000&sec_user_id=MS4wLjAB&aid=6383&device_platform=web&browser_language=zh-CN&browser_platform=Win32&browser_name=Mozilla&browser_version=5.0
    let url_params = [
        ("type_id", "0"),
        ("live_id", "1"),
        ("version_code", "99.99.99"),
        ("app_id", "1128"),
        ("room_id", &room_id_str),
        ("sec_user_id", sec_user_id),
        ("aid", "6383"),
        ("device_platform", "web"),
    ];

    // 构建URL
    let query_string = url_params
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let url = format!("https://webcast.amemv.com/webcast/room/reflow/info/?{query_string}");

    let mut headers = generate_user_agent_header();
    headers.insert("Referer", "https://live.douyin.com/".parse().unwrap());
    headers.insert("Cookie", account.cookies.clone().parse().unwrap());

    let resp = client.get(&url).headers(headers).send().await?;

    let status = resp.status();
    let text = resp.text().await?;

    if status.is_success() {
        // Try to parse as H5 response format
        if let Ok(h5_data) =
            serde_json::from_str::<super::response::DouyinH5RoomInfoResponse>(&text)
        {
            // Extract RoomBasicInfo from H5 response
            let room = &h5_data.data.room;
            let owner = &room.owner;

            let cover = room
                .cover
                .as_ref()
                .and_then(|c| c.url_list.first().cloned());
            let hls_url = room
                .stream_url
                .as_ref()
                .map(|s| s.hls_pull_url.clone())
                .unwrap_or_default();

            return Ok(DouyinBasicRoomInfo {
                room_id_str: room.id_str.clone(),
                room_title: room.title.clone(),
                cover,
                status: if room.status == 2 { 0 } else { 1 },
                hls_url,
                user_name: owner.nickname.clone(),
                user_avatar: owner
                    .avatar_thumb
                    .url_list
                    .first()
                    .unwrap_or(&String::new())
                    .clone(),
                sec_user_id: owner.sec_uid.clone(),
                stream_data: room
                    .stream_url
                    .as_ref()
                    .map(|s| s.live_core_sdk_data.pull_data.stream_data.clone())
                    .unwrap_or_default(),
            });
        }

        // If that fails, try to parse as a generic JSON to see what we got
        if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&text) {
            // Check if it's an error response
            if let Some(status_code) = json_value
                .get("status_code")
                .and_then(serde_json::Value::as_i64)
            {
                if status_code != 0 {
                    let error_msg = json_value
                        .get("data")
                        .and_then(|v| v.get("message").and_then(|v| v.as_str()))
                        .unwrap_or("Unknown error");

                    if status_code == 10011 {
                        return Err(RecorderError::ApiError {
                            error: error_msg.to_string(),
                        });
                    }

                    return Err(RecorderError::ApiError {
                        error: format!(
                            "API returned error status_code: {status_code} - {error_msg}"
                        ),
                    });
                }
            }

            // 检查是否是"invalid session"错误
            if let Some(status_message) = json_value.get("status_message").and_then(|v| v.as_str())
            {
                if status_message.contains("invalid session") {
                    return Err(RecorderError::ApiError { error:
                            "Invalid session - please check your cookies. Make sure you have valid sessionid, passport_csrf_token, and other authentication cookies from douyin.com".to_string(),
                        });
                }
            }

            return Err(RecorderError::ApiError {
                error: format!("Failed to parse h5 room info response: {text}"),
            });
        }
        log::error!("Failed to parse h5 room info response: {text}");
        return Err(RecorderError::ApiError {
            error: format!("Failed to parse h5 room info response: {text}"),
        });
    }

    log::error!("Failed to get h5 room info: {status}");
    Err(RecorderError::ApiError {
        error: format!("Failed to get h5 room info: {status} {text}"),
    })
}

pub async fn get_user_info(
    client: &Client,
    account: &Account,
) -> Result<super::response::User, RecorderError> {
    // Use the IM spotlight relation API to get user info
    let url = "https://www.douyin.com/aweme/v1/web/im/spotlight/relation/";
    let mut headers = generate_user_agent_header();
    headers.insert("Referer", "https://www.douyin.com/".parse().unwrap());
    headers.insert("Cookie", account.cookies.clone().parse().unwrap());

    let resp = client.get(url).headers(headers).send().await?;

    let status = resp.status();
    let text = resp.text().await?;

    if status.is_success() {
        if let Ok(data) = serde_json::from_str::<super::response::DouyinRelationResponse>(&text) {
            if data.status_code == 0 {
                let owner_sec_uid = &data.owner_sec_uid;

                // Find the user's own info in the followings list by matching sec_uid
                if let Some(followings) = &data.followings {
                    for following in followings {
                        if following.sec_uid == *owner_sec_uid {
                            let user = super::response::User {
                                id_str: following.uid.clone(),
                                sec_uid: following.sec_uid.clone(),
                                nickname: following.nickname.clone(),
                                avatar_thumb: following.avatar_thumb.clone(),
                                follow_info: super::response::FollowInfo::default(),
                                foreign_user: 0,
                                open_id_str: String::new(),
                            };
                            return Ok(user);
                        }
                    }
                }

                // If not found in followings, create a minimal user info from owner_sec_uid
                let user = super::response::User {
                    id_str: String::new(), // We don't have the numeric UID
                    sec_uid: owner_sec_uid.clone(),
                    nickname: "抖音用户".to_string(), // Default nickname
                    avatar_thumb: super::response::AvatarThumb { url_list: vec![] },
                    follow_info: super::response::FollowInfo::default(),
                    foreign_user: 0,
                    open_id_str: String::new(),
                };
                return Ok(user);
            }
        } else {
            log::error!("Failed to parse user info response: {text}");
            return Err(RecorderError::ApiError {
                error: format!("Failed to parse user info response: {text}"),
            });
        }
    }

    log::error!("Failed to get user info: {status}");

    Err(RecorderError::ApiError {
        error: format!("Failed to get user info: {status} {text}"),
    })
}

/// Download file from url to path
pub async fn download_file(client: &Client, url: &str, path: &Path) -> Result<(), RecorderError> {
    if !path.parent().unwrap().exists() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    }
    let response = client.get(url).send().await?;
    let bytes = response.bytes().await?;
    let mut file = tokio::fs::File::create(&path).await?;
    let mut content = std::io::Cursor::new(bytes);
    tokio::io::copy(&mut content, &mut file).await?;
    Ok(())
}
