use crate::account::Account;
use crate::platforms::huya::extractor::StreamInfo;
use crate::utils::user_agent_generator;
use crate::RoomInfo;
use crate::UserInfo;

use super::errors::HuyaClientError;

use reqwest::Client;
use scraper::Html;
use scraper::Selector;
use std::path::Path;

fn generate_user_agent_header() -> reqwest::header::HeaderMap {
    let user_agent = user_agent_generator::UserAgentGenerator::new().generate(true);
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("user-agent", user_agent.parse().unwrap());
    headers
}

pub async fn get_user_info(
    client: &Client,
    account: &Account,
) -> Result<UserInfo, HuyaClientError> {
    // https://m.huya.com/video/u/2246697169
    let mut headers = generate_user_agent_header();
    if let Ok(cookies) = account.cookies.parse() {
        headers.insert("cookie", cookies);
    } else {
        return Err(HuyaClientError::InvalidCookie);
    }
    let url = format!("https://m.huya.com/video/u/{}", account.id);
    let response = client.get(url).headers(headers).send().await?;
    let raw_content = response.text().await?;
    // <div class="video-list-info">
    //     <div class="podcast-box clearfix">
    //         <img src="http://huyaimg.msstatic.com/avatar/1060/3f/0e6c0694867ef98e9f869589608ce3_180_135.jpg" alt="">
    //         <div class="podcast-info-intro">
    //             <h2>X inrea  丶</h2>
    //             <p></p>
    //         </div>
    //     </div>
    // </div>
    let document = Html::parse_document(&raw_content);

    let avatar_selector = Selector::parse(".video-list-info .podcast-box img").unwrap();
    let name_selector = Selector::parse(".video-list-info .podcast-info-intro h2").unwrap();

    // 提取 avatar (img src)
    let avatar = document
        .select(&avatar_selector)
        .next()
        .and_then(|img| img.value().attr("src"))
        .map(|src| src.to_string());

    // 提取 name (h2 text)
    let name = document
        .select(&name_selector)
        .next()
        .map(|h2| h2.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty());

    Ok(UserInfo {
        user_id: account.id.clone(),
        user_name: name.unwrap_or_default(),
        user_avatar: avatar.unwrap_or_default(),
    })
}

pub async fn get_room_info(
    client: &Client,
    account: &Account,
    room_id: &str,
) -> Result<(UserInfo, RoomInfo, StreamInfo), HuyaClientError> {
    let mut headers = generate_user_agent_header();
    if let Ok(cookies) = account.cookies.parse() {
        headers.insert("cookie", cookies);
    } else {
        return Err(HuyaClientError::InvalidCookie);
    }
    headers.insert("Referer", "https://m.huya.com/".parse().unwrap());
    let url = format!("https://m.huya.com/{room_id}");
    let response = client.get(url).headers(headers).send().await?;
    let raw_content = response.text().await?;
    let (user_info, room_info, stream_info) =
        super::extractor::LiveStreamExtractor::extract_infos(&raw_content)?;

    Ok((user_info, room_info, stream_info))
}

/// Download file from url to path
pub async fn download_file(client: &Client, url: &str, path: &Path) -> Result<(), HuyaClientError> {
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

pub async fn get_index_content(client: &Client, url: &str) -> Result<String, HuyaClientError> {
    let headers = generate_user_agent_header();
    let response = client.get(url).headers(headers).send().await?;

    if response.status().is_success() {
        Ok(response.text().await?)
    } else {
        log::error!("get_index_content failed: {}", response.status());
        Err(HuyaClientError::InvalidStream)
    }
}

#[cfg(test)]
mod tests {
    use crate::platforms::PlatformType;

    use super::*;

    #[tokio::test]
    async fn test_get_user_info() {
        let client = Client::new();
        let account = Account {
            platform: PlatformType::Huya.as_str().to_string(),
            id: "2246697169".to_string(),
            name: "X inrea  丶".to_string(),
            avatar: "https://huyaimg.msstatic.com/avatar/1060/3f/0e6c0694867ef98e9f869589608ce3_180_135.jpg".to_string(),
            csrf: "".to_string(),
            cookies: "".to_string(),
        };
        let user_info = get_user_info(&client, &account).await.unwrap();
        println!("{:?}", user_info);
    }

    #[tokio::test]
    async fn test_get_room_info() {
        // set log level to debug
        std::env::set_var("RUST_LOG", "debug");
        let _ = env_logger::try_init();
        let client = Client::new();
        let account = Account::default();
        let (user_info, room_info, stream_info) =
            get_room_info(&client, &account, "599934").await.unwrap();
        println!("{:?}", user_info);
        println!("{:?}", room_info);
        println!("{:?}", stream_info);

        // query index content
        let index_content = get_index_content(&client, &stream_info.hls_url)
            .await
            .unwrap();
        println!("{:?}", index_content);
    }
}
