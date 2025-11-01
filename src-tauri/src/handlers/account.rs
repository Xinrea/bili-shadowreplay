use std::str::FromStr;

use crate::database::account::AccountRow;
use crate::state::State;
use crate::state_type;
use chrono::Utc;
use recorder::platforms::bilibili::api::{QrInfo, QrStatus};
use recorder::platforms::{bilibili, douyin, huya, PlatformType};
use recorder::UserInfo;

use hyper::header::HeaderValue;
#[cfg(feature = "gui")]
use tauri::State as TauriState;

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_accounts(state: state_type!()) -> Result<super::AccountInfo, String> {
    let account_info = super::AccountInfo {
        accounts: state.db.get_accounts().await?,
    };
    Ok(account_info)
}

fn get_item_from_cookies(name: &str, cookies: &str) -> Result<String, String> {
    Ok(cookies
        .split(';')
        .map(str::trim)
        .find_map(|cookie| cookie.strip_prefix(format!("{name}=").as_str()))
        .ok_or_else(|| format!("Invalid cookies: missing {name}").to_string())?
        .to_string())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn add_account(
    state: state_type!(),
    platform: String,
    cookies: &str,
) -> Result<(), String> {
    // check if cookies is valid
    if let Err(e) = cookies.parse::<HeaderValue>() {
        return Err(format!("Invalid cookies: {e}"));
    }

    let platform = PlatformType::from_str(&platform).map_err(|_| "Invalid platform".to_string())?;

    let csrf = match platform {
        PlatformType::BiliBili => {
            cookies
                .split(';')
                .map(str::trim)
                .find_map(|cookie| -> Option<String> {
                    if cookie.starts_with("bili_jct=") {
                        let var_name = &"bili_jct=";
                        Some(cookie[var_name.len()..].to_string())
                    } else {
                        None
                    }
                })
        }
        _ => Some(String::new()),
    };

    // fetch basic account user info
    let client = reqwest::Client::new();
    let user_info = match platform {
        PlatformType::BiliBili => {
            // For Bilibili, extract numeric uid from cookies
            if csrf.is_none() {
                return Err("Invalid bilibili cookies".to_string());
            }
            let uid = get_item_from_cookies("DedeUserID", cookies)?;
            let tmp_account = AccountRow {
                platform: platform.as_str().to_string(),
                uid,
                name: String::new(),
                avatar: String::new(),
                csrf: csrf.clone().unwrap(),
                cookies: cookies.into(),
                created_at: Utc::now().to_rfc3339(),
            };
            match bilibili::api::get_user_info(&client, &tmp_account.to_account(), &tmp_account.uid)
                .await
            {
                Ok(user_info) => UserInfo {
                    user_id: user_info.user_id,
                    user_name: user_info.user_name,
                    user_avatar: user_info.user_avatar_url,
                },
                Err(e) => {
                    return Err(e.to_string());
                }
            }
        }
        PlatformType::Douyin => {
            let tmp_account = AccountRow {
                platform: platform.as_str().to_string(),
                uid: "".into(),
                name: String::new(),
                avatar: String::new(),
                csrf: "".into(),
                cookies: cookies.into(),
                created_at: Utc::now().to_rfc3339(),
            };

            match douyin::api::get_user_info(&client, &tmp_account.to_account()).await {
                Ok(user_info) => {
                    // For Douyin, use sec_uid as the primary identifier in id_str field
                    let avatar_url = user_info
                        .avatar_thumb
                        .url_list
                        .first()
                        .cloned()
                        .unwrap_or_default();

                    UserInfo {
                        user_id: user_info.sec_uid,
                        user_name: user_info.nickname,
                        user_avatar: avatar_url,
                    }
                }
                Err(e) => {
                    return Err(format!("Failed to get Douyin user info: {e}"));
                }
            }
        }
        PlatformType::Huya => {
            let user_id = get_item_from_cookies("yyuid", cookies)?;

            let tmp_account = AccountRow {
                platform: platform.as_str().to_string(),
                uid: user_id,
                name: String::new(),
                avatar: String::new(),
                csrf: "".into(),
                cookies: cookies.into(),
                created_at: Utc::now().to_rfc3339(),
            };

            match huya::api::get_user_info(&client, &tmp_account.to_account()).await {
                Ok(user_info) => UserInfo {
                    user_id: user_info.user_id,
                    user_name: user_info.user_name,
                    user_avatar: user_info.user_avatar,
                },
                Err(e) => {
                    return Err(format!("Failed to get Huya user info: {e}"));
                }
            }
        }
        PlatformType::Youtube => {
            // unsupported
            return Err("Unsupported platform".to_string());
        }
    };

    let account = AccountRow {
        platform: platform.as_str().to_string(),
        uid: user_info.user_id,
        name: user_info.user_name,
        avatar: user_info.user_avatar,
        csrf: csrf.unwrap(),
        cookies: cookies.into(),
        created_at: Utc::now().to_rfc3339(),
    };
    state.db.add_account(&account).await?;
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn remove_account(
    state: state_type!(),
    platform: String,
    uid: String,
) -> Result<(), String> {
    if platform == "bilibili" {
        let account = state.db.get_account(&platform, &uid).await?;
        let client = reqwest::Client::new();
        let _ = bilibili::api::logout(&client, &account.to_account()).await;
    }
    Ok(state.db.remove_account(&platform, &uid).await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_account_count(state: state_type!()) -> Result<u64, String> {
    Ok(state.db.get_accounts().await?.len() as u64)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_qr_status(_state: state_type!(), qrcode_key: &str) -> Result<QrStatus, ()> {
    let client = reqwest::Client::new();
    match bilibili::api::get_qr_status(&client, qrcode_key).await {
        Ok(qr_status) => Ok(qr_status),
        Err(_e) => Err(()),
    }
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_qr(_state: state_type!()) -> Result<QrInfo, ()> {
    let client = reqwest::Client::new();
    match bilibili::api::get_qr(&client).await {
        Ok(qr_info) => Ok(qr_info),
        Err(_e) => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_item_from_cookies() {
        let cookies = "DedeUserID=1234567890; bili_jct=1234567890; yyuid=1234567890";
        let uid = get_item_from_cookies("DedeUserID", cookies).unwrap();
        assert_eq!(uid, "1234567890");
        let uid = get_item_from_cookies("yyuid", cookies).unwrap();
        assert_eq!(uid, "1234567890");
        let uid = get_item_from_cookies("bili_jct", cookies).unwrap();
        assert_eq!(uid, "1234567890");
        let uid = get_item_from_cookies("unknown", cookies).unwrap_err();
        assert_eq!(uid, "Invalid cookies: missing unknown");
    }
}
