use serde::{Deserialize, Serialize};

use crate::errors::ApiCollectionError;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QrInfo {
    pub oauth_key: String,
    pub url: String,
}

pub async fn get_qr(client: &reqwest::Client) -> Result<QrInfo, ApiCollectionError> {
    let res: serde_json::Value = client
        .get("https://passport.bilibili.com/x/passport-login/web/qrcode/generate")
        .send()
        .await?
        .json()
        .await?;
    Ok(QrInfo {
        oauth_key: res["data"]["qrcode_key"]
            .as_str()
            .ok_or(ApiCollectionError::InvalidValue {
                key: "qrcode_key".to_string(),
                value: "".to_string(),
            })?
            .to_string(),
        url: res["data"]["url"]
            .as_str()
            .ok_or(ApiCollectionError::InvalidValue {
                key: "url".to_string(),
                value: "".to_string(),
            })?
            .to_string(),
    })
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QrStatus {
    pub code: u8,
    pub cookies: String,
}

pub async fn get_qr_status(
    client: &reqwest::Client,
    qrcode_key: &str,
) -> Result<QrStatus, ApiCollectionError> {
    let res: serde_json::Value = client
        .get(format!(
            "https://passport.bilibili.com/x/passport-login/web/qrcode/poll?qrcode_key={}",
            qrcode_key
        ))
        .send()
        .await?
        .json()
        .await?;
    let code: u8 = res["data"]["code"].as_u64().unwrap_or(400) as u8;
    let mut cookies: String = "".to_string();
    if code == 0 {
        let url = res["data"]["url"]
            .as_str()
            .ok_or(ApiCollectionError::InvalidValue {
                key: "url".to_string(),
                value: "".to_string(),
            })?
            .to_string();
        let query_str = url.split('?').next_back().unwrap();
        cookies = query_str.replace('&', ";");
    }
    Ok(QrStatus { code, cookies })
}

pub async fn logout(client: &reqwest::Client, csrf: &str) -> Result<(), ApiCollectionError> {
    let url = "https://passport.bilibili.com/login/exit/v2";
    let params = [("csrf", csrf.to_string())];
    let _ = client
        .post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&params)
        .send()
        .await?;
    Ok(())
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UserInfo {
    pub user_id: u64,
    pub user_name: String,
    pub user_sign: String,
    pub user_avatar_url: String,
}

pub async fn get_user_info(
    client: &reqwest::Client,
    user_id: u64,
) -> Result<UserInfo, ApiCollectionError> {
    let params: serde_json::Value = serde_json::json!({
        "mid": user_id.to_string(),
        "platform": "web",
        "web_location": "1550101",
        "token": "",
        "w_webid": "",
    });
    let params = crate::bilibili::general::get_sign(client, params).await?;
    let resp = client
        .get(format!(
            "https://api.bilibili.com/x/space/wbi/acc/info?{}",
            params
        ))
        .send()
        .await?;

    if !resp.status().is_success() {
        if resp.status() == reqwest::StatusCode::PRECONDITION_FAILED {
            return Err(ApiCollectionError::RiskControlError);
        }
        return Err(ApiCollectionError::RequestError {
            err: resp.status().to_string(),
        });
    }

    let res: serde_json::Value = resp.json().await?;
    let code = res["code"]
        .as_u64()
        .ok_or(ApiCollectionError::InvalidValue {
            key: "code".to_string(),
            value: "".to_string(),
        })?;
    if code != 0 {
        return Err(ApiCollectionError::InvalidValue {
            key: "code".to_string(),
            value: code.to_string(),
        });
    }
    Ok(UserInfo {
        user_id,
        user_name: res["data"]["name"].as_str().unwrap_or("").to_string(),
        user_sign: res["data"]["sign"].as_str().unwrap_or("").to_string(),
        user_avatar_url: res["data"]["face"].as_str().unwrap_or("").to_string(),
    })
}
