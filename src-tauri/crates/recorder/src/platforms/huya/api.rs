use crate::account::Account;
use crate::platforms::huya::extractor::{PullUrl, StreamInfo};
use crate::utils::user_agent_generator;
use crate::RoomInfo;
use crate::UserInfo;

use super::errors::HuyaClientError;

use m3u8_rs::Playlist;
use reqwest::Client;
use scraper::Html;
use scraper::Selector;

fn generate_user_agent_header() -> reqwest::header::HeaderMap {
    let user_agent = user_agent_generator::UserAgentGenerator::new().generate(true);
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "user-agent",
        user_agent
            .parse()
            .expect("generated user agent is a valid header value"),
    );
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

    let avatar_selector = Selector::parse(".video-list-info .podcast-box img")
        .expect("avatar selector is a valid literal");
    let name_selector = Selector::parse(".video-list-info .podcast-info-intro h2")
        .expect("name selector is a valid literal");

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
    headers.insert(
        "Referer",
        reqwest::header::HeaderValue::from_static("https://m.huya.com/"),
    );
    let url = format!("https://m.huya.com/{room_id}");
    let response = client.get(url).headers(headers).send().await?;
    let raw_content = response.text().await?;
    let (user_info, room_info, stream_info) =
        super::extractor::LiveStreamExtractor::extract_infos(&raw_content)?;

    Ok((user_info, room_info, stream_info))
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

/// Pick the first pull candidate that actually serves stream data.
///
/// All candidates point at the same stream on different CDN nodes. Nodes
/// behave differently over time: they reject even correctly signed requests
/// with 403, and the HLS dispatch may return master playlists whose variant
/// URLs are unusable. Every candidate is therefore verified before the
/// recording starts, in preference order.
pub async fn pick_pull_url(
    client: &Client,
    stream: &StreamInfo,
) -> Result<PullUrl, HuyaClientError> {
    for (index, candidate) in stream.candidates.iter().enumerate() {
        let usable = match candidate {
            PullUrl::Flv(url) => probe_flv_stream(client, url).await,
            PullUrl::Hls(url) => probe_hls_stream(client, url).await,
        };
        if usable {
            if index > 0 {
                log::info!("Huya stream candidate {index} is usable");
            }
            return Ok(candidate.clone());
        }
        log::warn!("Huya stream candidate {index} failed");
    }

    Err(HuyaClientError::InvalidStream)
}

/// Verify an FLV URL serves actual FLV data (`FLV` magic bytes).
async fn probe_flv_stream(client: &Client, url: &str) -> bool {
    let headers = generate_user_agent_header();
    let Ok(mut response) = client.get(url).headers(headers).send().await else {
        return false;
    };
    if !response.status().is_success() {
        log::warn!("Huya flv probe failed: {}", response.status());
        return false;
    }

    let mut head: Vec<u8> = Vec::new();
    while head.len() < 9 {
        match response.chunk().await {
            Ok(Some(chunk)) => head.extend_from_slice(&chunk),
            _ => break,
        }
    }
    head.starts_with(b"FLV")
}

/// Verify an HLS URL leads to a media playlist. The Huya dispatch may serve a
/// master playlist, so its variants are followed one level deep (the recorder
/// resolves master playlists the same way).
async fn probe_hls_stream(client: &Client, url: &str) -> bool {
    let Ok(content) = get_index_content(client, url).await else {
        return false;
    };
    if content.contains("#EXTINF") {
        return true;
    }

    let Ok((_, Playlist::MasterPlaylist(master))) = m3u8_rs::parse_playlist(content.as_bytes())
    else {
        return false;
    };

    for variant in &master.variants {
        let Ok(variant_content) =
            get_index_content(client, &resolve_hls_url(url, &variant.uri)).await
        else {
            continue;
        };
        if variant_content.contains("#EXTINF") {
            return true;
        }
    }

    false
}

fn resolve_hls_url(base_url: &str, uri: &str) -> String {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return uri.to_string();
    }
    url::Url::parse(base_url)
        .ok()
        .and_then(|base| base.join(uri).ok())
        .map(|joined| joined.to_string())
        .unwrap_or_else(|| uri.to_string())
}

#[cfg(test)]
mod tests {
    use crate::platforms::PlatformType;

    use super::*;

    #[tokio::test]
    #[ignore = "live Huya network smoke test; run manually with --ignored"]
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
    async fn test_extract_room_info_from_fixture() {
        // Use the checked-in m.huya.com fixture instead of hitting live Huya in CI.
        // Live rooms are often offline, which previously failed with RelativeUrlWithoutBase
        // when stream_info.hls_url was empty.
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/huya_room_page.html");
        let html = tokio::fs::read_to_string(&fixture_path)
            .await
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", fixture_path.display()));

        let (user_info, room_info, stream_info) =
            crate::platforms::huya::extractor::LiveStreamExtractor::extract_infos(&html)
                .expect("fixture should parse via LiveStreamExtractor");

        assert!(!user_info.user_id.is_empty());
        assert_eq!(room_info.platform, "huya");
        // Stream URL may be empty when the captured room was offline; parsing must still succeed.
        let _ = stream_info;
    }

    /// Live smoke test for the anticode recomputation and candidate probing:
    /// `pick_pull_url` must find a candidate that serves stream data.
    /// Requires a live room; run manually with `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore = "live Huya network smoke test; run manually with --ignored"]
    async fn test_pick_pull_url_live() {
        let _ = env_logger::try_init();
        let client = Client::new();
        let account = Account::default();
        let (user_info, room_info, stream_info) =
            get_room_info(&client, &account, "691406").await.unwrap();
        println!("{user_info:?} {room_info:?}");
        assert!(room_info.status, "room 691406 must be live for this test");
        assert!(!stream_info.candidates.is_empty());
        println!("candidates: {:?}", stream_info.candidates.len());

        let picked = pick_pull_url(&client, &stream_info).await.unwrap();
        println!("picked: {picked:?}");
    }

    /// Optional live-network smoke test. Ignored in CI because room availability varies.
    #[tokio::test]
    #[ignore = "live Huya network smoke test; run manually with --ignored"]
    async fn test_get_room_info_live() {
        let _ = env_logger::try_init();
        let client = Client::new();
        let account = Account::default();
        let (user_info, room_info, stream_info) =
            get_room_info(&client, &account, "599934").await.unwrap();
        println!("{:?}", user_info);
        println!("{:?}", room_info);
        println!("{:?}", stream_info);
    }
}
