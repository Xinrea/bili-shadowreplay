use crate::database::account::AccountRow;
use crate::recorder::bilibili::{QrInfo, QrStatus};
use crate::state::State;
use tauri::State as TauriState;

#[tauri::command]
pub async fn get_accounts(state: TauriState<'_, State>) -> Result<super::AccountInfo, String> {
    let config = state.config.read().await.clone();
    let account_info = super::AccountInfo {
        primary_uid: config.primary_uid,
        accounts: state.db.get_accounts().await?,
    };
    Ok(account_info)
}

#[tauri::command]
pub async fn add_account(state: TauriState<'_, State>, cookies: &str) -> Result<AccountRow, String> {
    let mut is_primary = false;
    if state.config.read().await.primary_uid == 0 || state.db.get_accounts().await?.is_empty() {
        is_primary = true;
    }
    let account = state.db.add_account(cookies).await?;
    if is_primary {
        state.config.write().await.webid = state.client.fetch_webid(&account).await?;
        state.config.write().await.webid_ts = chrono::Utc::now().timestamp();
        state.config.write().await.primary_uid = account.uid;
    }
    let account_info = state
        .client
        .get_user_info(&state.config.read().await.webid, &account, account.uid)
        .await?;
    state
        .db
        .update_account(
            account_info.user_id,
            &account_info.user_name,
            &account_info.user_avatar_url,
        )
        .await?;
    Ok(account)
}

#[tauri::command]
pub async fn remove_account(state: TauriState<'_, State>, uid: u64) -> Result<(), String> {
    if state.db.get_accounts().await?.len() == 1 {
        return Err("At least one account is required".into());
    }
    // logout
    let account = state.db.get_account(uid).await?;
    state.client.logout(&account).await?;
    Ok(state.db.remove_account(uid).await?)
}

#[tauri::command]
pub async fn get_account_count(state: TauriState<'_, State>) -> Result<u64, String> {
    Ok(state.db.get_accounts().await?.len() as u64)
}

#[tauri::command]
pub async fn set_primary(state: TauriState<'_, State>, uid: u64) -> Result<(), String> {
    if (state.db.get_account(uid).await).is_ok() {
        state.config.write().await.primary_uid = uid;
        Ok(())
    } else {
        Err("Account not exist".into())
    }
} 

#[tauri::command]
pub async fn get_qr_status(state: tauri::State<'_, State>, qrcode_key: &str) -> Result<QrStatus, ()> {
    match state.client.get_qr_status(qrcode_key).await {
        Ok(qr_status) => Ok(qr_status),
        Err(_e) => Err(()),
    }
}

#[tauri::command]
pub async fn get_qr(state: tauri::State<'_, State>) -> Result<QrInfo, ()> {
    match state.client.get_qr().await {
        Ok(qr_info) => Ok(qr_info),
        Err(_e) => Err(()),
    }
}