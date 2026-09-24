use std::time::Duration;

use crate::errors::RecorderError;
use reqwest::Client;
use serde_json::{json, Value};
use url::Url;

/// Twitch's web player client id. Twitch's public web GraphQL endpoint requires
/// a client id even when the request does not use an authenticated account.
const TWITCH_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
const TWITCH_GQL_ENDPOINT: &str = "https://gql.twitch.tv/gql";
const TWITCH_USHER_ENDPOINT: &str = "https://usher.ttvnw.net/api/channel/hls";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

const ROOM_QUERY: &str = r#"query TwitchRoom($login: String!) {
  user(login: $login) {
    id
    login
    displayName
    profileImageURL(width: 300)
    stream {
      id
      title
      previewImageURL(width: 640, height: 360)
    }
  }
}"#;

const PLAYBACK_TOKEN_QUERY: &str = r#"query PlaybackAccessToken_Template($login: String!, $isLive: Boolean!, $vodID: ID!, $isVod: Boolean!, $playerType: String!) {
  streamPlaybackAccessToken(channelName: $login, params: {platform: "web", playerBackend: "mediaplayer", playerType: $playerType}) @include(if: $isLive) {
    value
    signature
  }
  videoPlaybackAccessToken(id: $vodID, params: {platform: "web", playerBackend: "mediaplayer", playerType: $playerType}) @include(if: $isVod) {
    value
    signature
  }
}"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomInfo {
    pub live: bool,
    pub live_id: Option<String>,
    pub title: String,
    pub cover: String,
    pub user_id: String,
    pub user_name: String,
    pub user_avatar: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamInfo {
    pub hls_url: String,
    pub expires: i64,
}

/// Normalize a Twitch channel name or Twitch URL using the shared chat-provider
/// implementation so recording and IRC subscribe to exactly the same channel.
pub fn normalize_channel(input: &str) -> Result<String, RecorderError> {
    danmu_stream::provider::normalize_twitch_channel(input)
        .map_err(|error| RecorderError::ApiError { error })
}

async fn graphql_at(
    client: &Client,
    endpoint: &str,
    request: Value,
) -> Result<Value, RecorderError> {
    let response = client
        .post(endpoint)
        .header("Client-ID", TWITCH_CLIENT_ID)
        .header("Content-Type", "application/json")
        .header("User-Agent", "BiliBili-ShadowReplay")
        .timeout(REQUEST_TIMEOUT)
        .json(&request)
        .send()
        .await?;
    let status = response.status();
    let body = response.json::<Value>().await?;

    if !status.is_success() {
        return Err(RecorderError::ApiError {
            error: format!("Twitch GraphQL returned {status}: {body}"),
        });
    }

    let body = match body {
        Value::Array(responses) => responses.into_iter().next().unwrap_or(Value::Null),
        body => body,
    };
    if let Some(errors) = body.get("errors").and_then(Value::as_array) {
        let message = errors
            .iter()
            .filter_map(|error| error.get("message").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(RecorderError::ApiError {
            error: if message.is_empty() {
                format!("Twitch GraphQL request failed: {errors:?}")
            } else {
                message
            },
        });
    }

    body.get("data")
        .cloned()
        .ok_or_else(|| RecorderError::InvalidResponseJson { resp: body.clone() })
}

pub async fn get_room_info(client: &Client, channel: &str) -> Result<RoomInfo, RecorderError> {
    get_room_info_at(client, channel, TWITCH_GQL_ENDPOINT).await
}

async fn get_room_info_at(
    client: &Client,
    channel: &str,
    graphql_endpoint: &str,
) -> Result<RoomInfo, RecorderError> {
    let channel = normalize_channel(channel)?;
    let data = graphql_at(
        client,
        graphql_endpoint,
        json!([{
            "operationName": "TwitchRoom",
            "query": ROOM_QUERY,
            "variables": { "login": channel.clone() },
        }]),
    )
    .await?;

    let user =
        data.get("user")
            .and_then(Value::as_object)
            .ok_or_else(|| RecorderError::ApiError {
                error: format!("Twitch channel not found: {channel}"),
            })?;

    let user_id = string_field(user.get("id"), "user id")?;
    let user_name = user
        .get("displayName")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .or_else(|| user.get("login").and_then(Value::as_str))
        .ok_or_else(|| RecorderError::ApiError {
            error: "Twitch response is missing display name".to_string(),
        })?
        .to_string();
    let user_avatar = optional_string(user.get("profileImageURL")).unwrap_or_default();
    let stream = user.get("stream").filter(|value| !value.is_null());

    let (live, live_id, title, cover) = if let Some(stream) = stream {
        (
            true,
            Some(string_field(stream.get("id"), "stream id")?),
            optional_string(stream.get("title")).unwrap_or_else(|| user_name.clone()),
            optional_string(stream.get("previewImageURL")).unwrap_or_else(|| user_avatar.clone()),
        )
    } else {
        (false, None, user_name.clone(), user_avatar.clone())
    };

    Ok(RoomInfo {
        live,
        live_id,
        title,
        cover,
        user_id,
        user_name,
        user_avatar,
    })
}

pub async fn get_stream_url(client: &Client, channel: &str) -> Result<StreamInfo, RecorderError> {
    get_stream_url_at(client, channel, TWITCH_GQL_ENDPOINT, TWITCH_USHER_ENDPOINT).await
}

async fn get_stream_url_at(
    client: &Client,
    channel: &str,
    graphql_endpoint: &str,
    usher_endpoint: &str,
) -> Result<StreamInfo, RecorderError> {
    let channel = normalize_channel(channel)?;
    let data = graphql_at(
        client,
        graphql_endpoint,
        json!([{
            "operationName": "PlaybackAccessToken_Template",
            "query": PLAYBACK_TOKEN_QUERY,
            "variables": {
                "isLive": true,
                "login": channel.clone(),
                "isVod": false,
                "vodID": "",
                "playerType": "site",
            },
        }]),
    )
    .await?;

    let token = data
        .get("streamPlaybackAccessToken")
        .and_then(Value::as_object)
        .ok_or_else(|| RecorderError::NoStreamAvailable)?;
    let token_value = token
        .get("value")
        .and_then(Value::as_str)
        .ok_or_else(|| RecorderError::InvalidResponse)?;
    let signature = token
        .get("signature")
        .and_then(Value::as_str)
        .ok_or_else(|| RecorderError::InvalidResponse)?;
    let token_json: Value =
        serde_json::from_str(token_value).map_err(|_| RecorderError::InvalidResponseJson {
            resp: json!({"streamPlaybackAccessToken": "<invalid>"}),
        })?;
    let expires = token_json
        .get("expires")
        .and_then(Value::as_i64)
        .unwrap_or_default();

    let mut url = Url::parse(&format!("{usher_endpoint}/{channel}.m3u8"))
        .map_err(|_| RecorderError::InvalidResponse)?;
    url.query_pairs_mut()
        .append_pair("client_id", TWITCH_CLIENT_ID)
        .append_pair("token", token_value)
        .append_pair("sig", signature)
        .append_pair("allow_source", "true")
        .append_pair("allow_audio_only", "true")
        .append_pair("player_backend", "mediaplayer")
        .append_pair("player_type", "site")
        .append_pair("p", &fastrand::u32(..1_000_000).to_string());

    let master_url = url.to_string();
    Ok(StreamInfo {
        hls_url: select_best_variant(client, &master_url).await?,
        expires,
    })
}

/// Twitch returns a master playlist even when the source stream is available.
/// Pick the highest-bandwidth variant here because the shared HLS recorder
/// expects a media playlist and otherwise defaults to Twitch's first (usually
/// 160p) rendition.
async fn select_best_variant(client: &Client, master_url: &str) -> Result<String, RecorderError> {
    let response = client
        .get(master_url)
        .header("User-Agent", "BiliBili-ShadowReplay")
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() {
        return Err(RecorderError::ApiError {
            error: format!("Twitch HLS returned {status}"),
        });
    }

    let (_, playlist) = m3u8_rs::parse_playlist(&bytes).map_err(|_| {
        // Playlist responses contain short-lived signed URLs; don't echo their
        // contents into errors or logs.
        RecorderError::M3u8ParseFailed {
            content: "Twitch returned an invalid HLS playlist".to_string(),
        }
    })?;
    match playlist {
        m3u8_rs::Playlist::MediaPlaylist(_) => Ok(master_url.to_string()),
        m3u8_rs::Playlist::MasterPlaylist(playlist) => {
            let variant = playlist
                .variants
                .iter()
                .max_by_key(|variant| variant.bandwidth)
                .ok_or_else(|| RecorderError::M3u8ParseFailed {
                    content: "Twitch HLS master playlist has no variants".to_string(),
                })?;
            let base = Url::parse(master_url).map_err(|_| RecorderError::InvalidResponse)?;
            base.join(&variant.uri)
                .map(|url| url.to_string())
                .map_err(|_| RecorderError::InvalidResponse)
        }
    }
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn string_field(value: Option<&Value>, field: &str) -> Result<String, RecorderError> {
    optional_string(value).ok_or_else(|| RecorderError::ApiError {
        error: format!("Twitch response is missing {field}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn normalize_channel_accepts_logins_handles_and_urls() {
        assert_eq!(normalize_channel("Ninja").unwrap(), "ninja");
        assert_eq!(normalize_channel("@Ninja").unwrap(), "ninja");
        assert_eq!(
            normalize_channel("https://www.twitch.tv/Ninja?ref=foo").unwrap(),
            "ninja"
        );
        assert_eq!(
            normalize_channel("www.twitch.tv/Ninja/live").unwrap(),
            "ninja"
        );
        assert_eq!(normalize_channel("m.twitch.tv/@Ninja").unwrap(), "ninja");
    }

    #[test]
    fn normalize_channel_rejects_invalid_logins() {
        assert!(normalize_channel("").is_err());
        assert!(normalize_channel("https://www.twitch.tv/").is_err());
        assert!(normalize_channel("bad channel").is_err());
        assert!(normalize_channel("a/b/c").is_err());
        assert!(normalize_channel("https://example.com/ninja").is_err());
        assert!(normalize_channel("https://www.twitch.tv/directory/all").is_err());
        assert!(normalize_channel("ninja/live").is_err());
        assert!(normalize_channel("https://www.twitch.tv/ninja/live/extra").is_err());
    }

    #[tokio::test]
    async fn selects_highest_bandwidth_hls_variant() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/master.m3u8"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=160000,RESOLUTION=426x240\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=6000000,RESOLUTION=1920x1080\nhigh.m3u8\n",
            ))
            .mount(&server)
            .await;

        let selected =
            select_best_variant(&Client::new(), &format!("{}/master.m3u8", server.uri()))
                .await
                .unwrap();
        assert_eq!(selected, format!("{}/high.m3u8", server.uri()));
    }

    #[tokio::test]
    async fn selects_highest_bandwidth_absolute_hls_variant() {
        let server = MockServer::start().await;
        let low_variant = "https://low.example/low.m3u8";
        let high_variant = "https://high.example/high.m3u8";
        Mock::given(method("GET"))
            .and(path("/master-absolute.m3u8"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=160000\n{low_variant}\n#EXT-X-STREAM-INF:BANDWIDTH=6000000\n{high_variant}\n"
            )))
            .mount(&server)
            .await;

        let selected = select_best_variant(
            &Client::new(),
            &format!("{}/master-absolute.m3u8", server.uri()),
        )
        .await
        .unwrap();
        assert_eq!(selected, high_variant);
    }

    #[tokio::test]
    async fn graphql_room_query_parses_live_room_metadata() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/gql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "data": {
                        "user": {
                            "id": "123",
                            "login": "ninja",
                            "displayName": "Ninja",
                            "profileImageURL": "https://example.test/avatar.jpg",
                            "stream": {
                                "id": "456",
                                "title": "Live title",
                                "previewImageURL": "https://example.test/cover.jpg"
                            }
                        }
                    }
                }
            ])))
            .mount(&server)
            .await;

        let room = get_room_info_at(&Client::new(), "ninja", &format!("{}/gql", server.uri()))
            .await
            .unwrap();
        assert!(room.live);
        assert_eq!(room.live_id.as_deref(), Some("456"));
        assert_eq!(room.user_name, "Ninja");
        assert_eq!(room.title, "Live title");
        assert_eq!(room.cover, "https://example.test/cover.jpg");
    }

    #[tokio::test]
    async fn graphql_room_query_handles_null_user() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/gql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "data": { "user": null } }
            ])))
            .mount(&server)
            .await;

        let result =
            get_room_info_at(&Client::new(), "ninja", &format!("{}/gql", server.uri())).await;
        assert!(matches!(result, Err(RecorderError::ApiError { .. })));
    }

    #[tokio::test]
    async fn playback_token_expiry_is_parsed_as_unix_seconds() {
        let server = MockServer::start().await;
        let expires = 1_800_000_000_i64;
        let token = serde_json::json!({ "expires": expires }).to_string();
        Mock::given(method("POST"))
            .and(path("/gql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "data": {
                        "streamPlaybackAccessToken": {
                            "value": token,
                            "signature": "test-signature"
                        }
                    }
                }
            ])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/hls/ninja.m3u8"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6.0,\nsegment.ts\n#EXT-X-ENDLIST\n",
            ))
            .mount(&server)
            .await;

        let stream = get_stream_url_at(
            &Client::new(),
            "ninja",
            &format!("{}/gql", server.uri()),
            &format!("{}/hls", server.uri()),
        )
        .await
        .unwrap();
        assert_eq!(stream.expires, expires);
        assert!(stream
            .hls_url
            .starts_with(&format!("{}/hls/ninja.m3u8?", server.uri())));
    }

    #[tokio::test]
    async fn missing_playback_token_returns_no_stream() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/gql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "data": {} }
            ])))
            .mount(&server)
            .await;

        let result = get_stream_url_at(
            &Client::new(),
            "ninja",
            &format!("{}/gql", server.uri()),
            &format!("{}/hls", server.uri()),
        )
        .await;
        assert!(matches!(result, Err(RecorderError::NoStreamAvailable)));
    }

    #[tokio::test]
    async fn accepts_a_media_playlist_without_rewriting_its_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/media.m3u8"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6.0,\nsegment.ts\n#EXT-X-ENDLIST\n",
            ))
            .mount(&server)
            .await;

        let media_url = format!("{}/media.m3u8?token=one", server.uri());
        let selected = select_best_variant(&Client::new(), &media_url)
            .await
            .unwrap();
        assert_eq!(selected, media_url);
    }

    #[test]
    fn playback_query_contains_live_token_fields() {
        assert!(PLAYBACK_TOKEN_QUERY.contains("streamPlaybackAccessToken"));
        assert!(PLAYBACK_TOKEN_QUERY.contains("videoPlaybackAccessToken"));
        assert!(PLAYBACK_TOKEN_QUERY.contains("playerBackend"));
    }
}
