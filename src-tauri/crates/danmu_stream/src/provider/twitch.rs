use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use log::{debug, info, warn};
use tokio::sync::RwLock;
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::provider::{DanmuMessageType, DanmuProvider};
use crate::{DanmuMessage, DanmuStreamError};

const TWITCH_IRC_URL: &str = "wss://irc-ws.chat.twitch.tv:443";
const RETRY_DELAY: Duration = Duration::from_secs(5);
const USER_AGENT: &str = "BiliBili-ShadowReplay";

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type WsWrite = futures_util::stream::SplitSink<WsStream, WsMessage>;

/// Anonymous Twitch IRC is sufficient for reading public channel chat. It
/// avoids requiring every recorder to configure a Twitch account or OAuth
/// token, while still receiving the same PRIVMSG events as the web client.
pub struct TwitchDanmu {
    channel: String,
    stop: Arc<RwLock<bool>>,
    write: Arc<RwLock<Option<WsWrite>>>,
}

#[async_trait]
impl DanmuProvider for TwitchDanmu {
    async fn new(_identifier: &str, room_id: &str) -> Result<Self, DanmuStreamError> {
        let channel = normalize_channel(room_id)
            .map_err(|err| DanmuStreamError::InvalidIdentifier { err })?;
        Ok(Self {
            channel,
            stop: Arc::new(RwLock::new(false)),
            write: Arc::new(RwLock::new(None)),
        })
    }

    async fn start(
        &self,
        tx: tokio::sync::mpsc::UnboundedSender<DanmuMessageType>,
    ) -> Result<(), DanmuStreamError> {
        let mut retry_count = 0;
        loop {
            if *self.stop.read().await {
                return Ok(());
            }

            match self.connect_and_handle(tx.clone()).await {
                Ok(()) => {
                    retry_count = 0;
                    debug!("Twitch IRC connection closed for {}", self.channel);
                }
                Err(error) => {
                    retry_count += 1;
                    warn!(
                        "Twitch IRC connection failed for {} (attempt {}): {}",
                        self.channel, retry_count, error
                    );
                }
            }

            if *self.stop.read().await {
                return Ok(());
            }
            sleep(RETRY_DELAY).await;
        }
    }

    async fn stop(&self) -> Result<(), DanmuStreamError> {
        *self.stop.write().await = true;
        if let Some(mut write) = self.write.write().await.take() {
            let _ = write.close().await;
        }
        Ok(())
    }
}

impl TwitchDanmu {
    async fn connect_and_handle(
        &self,
        tx: tokio::sync::mpsc::UnboundedSender<DanmuMessageType>,
    ) -> Result<(), DanmuStreamError> {
        let mut request = TWITCH_IRC_URL
            .into_client_request()
            .map_err(|error| websocket_error(error.to_string()))?;
        request
            .headers_mut()
            .insert("User-Agent", HeaderValue::from_static(USER_AGENT));

        let (socket, _) = connect_async(request)
            .await
            .map_err(|error| websocket_error(error.to_string()))?;
        let (write, mut read) = socket.split();
        *self.write.write().await = Some(write);

        // Twitch accepts SCHMOOPII as the anonymous password. The generated
        // justinfan nick keeps separate recorder connections independent.
        let nick = format!("justinfan{}", 100_000 + rand::random::<u32>() % 900_000);
        self.send_line("CAP REQ :twitch.tv/tags twitch.tv/commands twitch.tv/membership")
            .await?;
        self.send_line("PASS SCHMOOPII").await?;
        self.send_line(&format!("NICK {nick}")).await?;
        self.send_line(&format!("USER {nick} 8 * :{nick}")).await?;
        self.send_line(&format!("JOIN #{}", self.channel)).await?;

        info!("Twitch IRC connected to {}", self.channel);
        while let Some(message) = read.next().await {
            let message = message.map_err(|error| websocket_error(error.to_string()))?;
            match message {
                WsMessage::Text(text) => {
                    for line in text.lines() {
                        if line.starts_with("PING") {
                            let payload = line.strip_prefix("PING").unwrap_or_default();
                            self.send_line(&format!("PONG{payload}")).await?;
                            continue;
                        }
                        if let Some(danmu) = parse_privmsg(line, &self.channel) {
                            let _ = tx.send(DanmuMessageType::DanmuMessage(danmu));
                        }
                    }
                }
                WsMessage::Ping(payload) => {
                    self.send_message(WsMessage::Pong(payload)).await?;
                }
                WsMessage::Close(_) => return Ok(()),
                _ => {}
            }
        }

        Ok(())
    }

    async fn send_line(&self, line: &str) -> Result<(), DanmuStreamError> {
        self.send_message(WsMessage::Text(format!("{line}\r\n").into()))
            .await
    }

    async fn send_message(&self, message: WsMessage) -> Result<(), DanmuStreamError> {
        let mut write = self.write.write().await;
        let Some(write) = write.as_mut() else {
            return Err(websocket_error("connection is closed"));
        };
        write
            .send(message)
            .await
            .map_err(|error| websocket_error(error.to_string()))
    }
}

fn websocket_error(error: impl Into<String>) -> DanmuStreamError {
    DanmuStreamError::WebsocketError { err: error.into() }
}

pub(super) fn normalize_channel(room_id: &str) -> Result<String, String> {
    let input = room_id.trim();
    let channel = if input.contains("://")
        || input.starts_with("www.twitch.tv/")
        || input.starts_with("m.twitch.tv/")
        || input.starts_with("twitch.tv/")
    {
        let url = if input.contains("://") {
            url::Url::parse(input)
        } else {
            url::Url::parse(&format!("https://{input}"))
        }
        .map_err(|_| invalid_channel(room_id))?;
        let host = url
            .host_str()
            .unwrap_or_default()
            .trim_start_matches("www.");
        if !matches!(host, "twitch.tv" | "m.twitch.tv") {
            return Err(invalid_channel(room_id));
        }
        let Some(segments) = url.path_segments() else {
            return Err(invalid_channel(room_id));
        };
        let mut segments = segments.filter(|segment| !segment.is_empty());
        let channel = segments.next().unwrap_or_default().trim_start_matches('@');
        let valid_path = match segments.next() {
            None => true,
            Some("live") => segments.next().is_none(),
            Some(_) => false,
        };
        if !valid_path {
            return Err(invalid_channel(room_id));
        }
        channel.to_string()
    } else {
        let login = input
            .trim_start_matches('#')
            .trim_start_matches('@')
            .split(['?', '#'])
            .next()
            .unwrap_or_default();
        if login.contains('/') {
            return Err(invalid_channel(room_id));
        }
        login.to_string()
    }
    .to_ascii_lowercase();

    if channel.is_empty()
        || channel.len() > 25
        || !channel
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(invalid_channel(room_id));
    }
    Ok(channel)
}

fn invalid_channel(room_id: &str) -> String {
    format!("Invalid Twitch channel: {room_id}")
}

fn parse_privmsg(line: &str, channel: &str) -> Option<DanmuMessage> {
    // The prefix separator (` :nick!…`) appears before the command, so split
    // at the PRIVMSG command first rather than using the first ` :` in the
    // line. The second separator is the start of the chat content.
    let command_start = line.find(" PRIVMSG #")?;
    let metadata = &line[..command_start];
    let command_and_message = &line[command_start + 1..];
    let (command, message) = command_and_message.split_once(" :")?;
    let mut command_parts = command.split_whitespace();
    if command_parts.next()? != "PRIVMSG" {
        return None;
    }
    let target = command_parts.next()?;
    if !target.trim_start_matches('#').eq_ignore_ascii_case(channel) {
        return None;
    }

    let (tags, prefix_start) = if let Some(tags) = metadata.strip_prefix('@') {
        let (tags, rest) = tags.split_once(' ')?;
        (parse_tags(tags), rest.trim_start())
    } else {
        (HashMap::new(), metadata)
    };

    let nick = tags
        .get("display-name")
        .filter(|name| !name.is_empty())
        .cloned()
        .or_else(|| {
            prefix_start
                .strip_prefix(':')
                .and_then(|prefix| prefix.split('!').next())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_string());
    let user_id = tags
        .get("user-id")
        .and_then(|user_id| user_id.parse::<u64>().ok())
        .unwrap_or_else(|| stable_user_id(&nick));
    let color = tags
        .get("color")
        .and_then(|color| u32::from_str_radix(color.trim_start_matches('#'), 16).ok())
        .unwrap_or_default();
    let timestamp = tags
        .get("tmi-sent-ts")
        .and_then(|timestamp| timestamp.parse::<i64>().ok())
        .unwrap_or_else(|| Utc::now().timestamp_millis());

    Some(DanmuMessage {
        room_id: channel.to_string(),
        user_id,
        user_name: nick,
        message: message.to_string(),
        color,
        timestamp,
    })
}

fn parse_tags(raw: &str) -> HashMap<String, String> {
    raw.split(';')
        .filter_map(|tag| {
            let (key, value) = tag.split_once('=')?;
            Some((key.to_string(), unescape_tag(value)))
        })
        .collect()
}

fn unescape_tag(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            result.push(match character {
                's' => ' ',
                ':' => ';',
                'r' => '\r',
                'n' => '\n',
                '\\' => '\\',
                other => other,
            });
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else {
            result.push(character);
        }
    }
    if escaped {
        result.push('\\');
    }
    result
}

fn stable_user_id(name: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_channel_logins_and_urls() {
        assert_eq!(normalize_channel("Ninja").unwrap(), "ninja");
        assert_eq!(normalize_channel("#Ninja").unwrap(), "ninja");
        assert_eq!(
            normalize_channel("https://www.twitch.tv/Ninja/live").unwrap(),
            "ninja"
        );
        assert!(normalize_channel("https://example.com/Ninja").is_err());
        assert!(normalize_channel("ninja/live").is_err());
    }

    #[test]
    fn parses_privmsg_tags_and_message() {
        let line = "@badge-info=;badges=;color=#1E90FF;display-name=Some\\sUser;user-id=42;tmi-sent-ts=1700000000123 :someuser!someuser@someuser.tmi.twitch.tv PRIVMSG #ninja :hello Twitch";
        let message = parse_privmsg(line, "ninja").unwrap();
        assert_eq!(message.room_id, "ninja");
        assert_eq!(message.user_id, 42);
        assert_eq!(message.user_name, "Some User");
        assert_eq!(message.message, "hello Twitch");
        assert_eq!(message.color, 0x1E90FF);
        assert_eq!(message.timestamp, 1_700_000_000_123);
    }

    #[test]
    fn ignores_messages_for_another_channel() {
        let line = ":user!user@user.tmi.twitch.tv PRIVMSG #other :hello";
        assert!(parse_privmsg(line, "ninja").is_none());
    }

    #[test]
    fn parses_tag_escaping() {
        let tags = parse_tags("display-name=Some\\sUser;note=a\\:b\\\\c");
        assert_eq!(tags.get("display-name").unwrap(), "Some User");
        assert_eq!(tags.get("note").unwrap(), "a;b\\c");
    }
}
