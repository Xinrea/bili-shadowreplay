use crate::database::account::AccountRow;
use crate::recorder::bilibili::api::{QrInfo, QrStatus};
use crate::recorder::{bilibili, douyin};
use crate::state::State;
use crate::state_type;

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

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn add_account(
    state: state_type!(),
    platform: String,
    cookies: &str,
) -> Result<AccountRow, String> {
    // check if cookies is valid
    if let Err(e) = cookies.parse::<HeaderValue>() {
        return Err(format!("Invalid cookies: {e}"));
    }
    let account = state.db.add_account(&platform, cookies).await?;
    let client = reqwest::Client::new();

    if platform == "bilibili" {
        let account_info = match bilibili::api::get_user_info(&client, &account, account.uid).await
        {
            Ok(account_info) => account_info,
            Err(e) => {
                return Err(e.to_string());
            }
        };
        state
            .db
            .update_account(
                &platform,
                account_info.user_id,
                &account_info.user_name,
                &account_info.user_avatar_url,
            )
            .await?;
        return Ok(account);
    }

    if platform == "douyin" {
        // Get user info from Douyin API
        match douyin::api::get_user_info(&client, &account).await {
            Ok(user_info) => {
                // For Douyin, use sec_uid as the primary identifier in id_str field
                let avatar_url = user_info
                    .avatar_thumb
                    .url_list
                    .first()
                    .cloned()
                    .unwrap_or_default();

                state
                    .db
                    .update_account_with_id_str(
                        &account,
                        &user_info.sec_uid,
                        &user_info.nickname,
                        &avatar_url,
                    )
                    .await?;
            }
            Err(e) => {
                log::warn!("Failed to get Douyin user info: {e}");
                // Keep the account but with default values
            }
        }

        return Ok(account);
    }

    // if platform == "huya" {
    //     let huya_client = crate::recorder::huya::client::HuyaClient::new();
    //     match huya_client.get_user_info(&account).await {
    //         Ok(user_info) => {
    //             state
    //                 .db
    //                 .update_account(
    //                     &platform,
    //                     user_info.user_id,
    //                     &user_info.user_name,
    //                     &user_info.user_avatar_url,
    //                 )
    //                 .await?;
    //         }
    //         Err(e) => {
    //             log::warn!("Failed to get Huya user info: {e}");
    //             // Keep the account but with default values
    //         }
    //     }
    //     return Ok(account);
    // }

    todo!("unsupported platform: {platform}");
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn remove_account(
    state: state_type!(),
    platform: String,
    uid: i64,
) -> Result<(), String> {
    if platform == "bilibili" {
        let account = state.db.get_account(&platform, uid).await?;
        let client = reqwest::Client::new();
        return match bilibili::api::logout(&client, &account).await {
            Ok(()) => Ok(()),
            Err(e) => Err(e.to_string()),
        };
    }
    Ok(state.db.remove_account(&platform, uid).await?)
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
