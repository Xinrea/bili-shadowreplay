use crate::database::account::AccountRow;
use crate::recorder::bilibili::client::{QrInfo, QrStatus};
use crate::state::State;
use tauri::State as TauriState;

#[tauri::command]
pub async fn get_accounts(state: TauriState<'_, State>) -> Result<super::AccountInfo, String> {
    let account_info = super::AccountInfo {
        accounts: state.db.get_accounts().await?,
    };
    Ok(account_info)
}

#[tauri::command]
pub async fn add_account(
    state: TauriState<'_, State>,
    platform: String,
    cookies: &str,
) -> Result<AccountRow, String> {
    let account = state.db.add_account(&platform, cookies).await?;
    if platform == "bilibili" {
        state.config.write().await.webid = state.client.fetch_webid(&account).await?;
        state.config.write().await.webid_ts = chrono::Utc::now().timestamp();
        let account_info = state
            .client
            .get_user_info(&state.config.read().await.webid, &account, account.uid)
            .await?;
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

#[tauri::command]
pub async fn remove_account(
    state: TauriState<'_, State>,
    platform: String,
    uid: u64,
) -> Result<(), String> {
    if platform == "bilibili" {
        let account = state.db.get_account(&platform, uid).await?;
        state.client.logout(&account).await?;
    }
    Ok(state.db.remove_account(&platform, uid).await?)
}

#[tauri::command]
pub async fn get_account_count(state: TauriState<'_, State>) -> Result<u64, String> {
    Ok(state.db.get_accounts().await?.len() as u64)
}

#[tauri::command]
pub async fn get_qr_status(
    state: tauri::State<'_, State>,
    qrcode_key: &str,
) -> Result<QrStatus, ()> {
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
