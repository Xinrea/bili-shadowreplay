use crate::database::account::AccountRow;
use crate::recorder::bilibili::client::{QrInfo, QrStatus};
use crate::state::State;
use crate::state_type;

#[cfg(not(feature = "headless"))]
use tauri::State as TauriState;

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn get_accounts(state: state_type!()) -> Result<super::AccountInfo, String> {
    let account_info = super::AccountInfo {
        accounts: state.db.get_accounts().await?,
    };
    Ok(account_info)
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn add_account(
    state: state_type!(),
    platform: String,
    cookies: &str,
) -> Result<AccountRow, String> {
    let account = state.db.add_account(&platform, cookies).await?;
    if platform == "bilibili" {
        let account_info = state.client.get_user_info(&account, account.uid).await?;
        state
            .db
            .update_account(
                &platform,
                account_info.user_id,
                &account_info.user_name,
                &account_info.user_avatar_url,
            )
            .await?;
    }
    Ok(account)
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn remove_account(
    state: state_type!(),
    platform: String,
    uid: u64,
) -> Result<(), String> {
    if platform == "bilibili" {
        let account = state.db.get_account(&platform, uid).await?;
        state.client.logout(&account).await?;
    }
    Ok(state.db.remove_account(&platform, uid).await?)
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn get_account_count(state: state_type!()) -> Result<u64, String> {
    Ok(state.db.get_accounts().await?.len() as u64)
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn get_qr_status(state: state_type!(), qrcode_key: &str) -> Result<QrStatus, ()> {
    match state.client.get_qr_status(qrcode_key).await {
        Ok(qr_status) => Ok(qr_status),
        Err(_e) => Err(()),
    }
}

#[cfg_attr(not(feature = "headless"), tauri::command)]
pub async fn get_qr(state: state_type!()) -> Result<QrInfo, ()> {
    match state.client.get_qr().await {
        Ok(qr_info) => Ok(qr_info),
        Err(_e) => Err(()),
    }
}
