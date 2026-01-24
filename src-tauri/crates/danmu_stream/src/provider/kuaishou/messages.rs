use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct SocketMessage {
    #[prost(enumeration = "PayloadType", tag = "1")]
    pub payload_type: i32,
    #[prost(enumeration = "CompressionType", tag = "2")]
    pub compression_type: i32,
    #[prost(bytes = "vec", tag = "3")]
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum PayloadType {
    Unknown = 0,
    CsHeartbeat = 1,
    CsEnterRoom = 200,
    ScFeedPush = 310,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum CompressionType {
    Unknown = 0,
    None = 1,
    Gzip = 2,
    Aes = 3,
}

#[derive(Clone, PartialEq, Message)]
pub struct CsHeartbeat {
    #[prost(uint64, tag = "1")]
    pub timestamp: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct CsWebEnterRoom {
    #[prost(string, tag = "1")]
    pub token: String,
    #[prost(string, tag = "2")]
    pub live_stream_id: String,
    #[prost(uint32, tag = "3")]
    pub reconnect_count: u32,
    #[prost(uint32, tag = "4")]
    pub last_error_code: u32,
    #[prost(string, tag = "5")]
    pub exp_tag: String,
    #[prost(string, tag = "6")]
    pub attach: String,
    #[prost(string, tag = "7")]
    pub page_id: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct ScWebFeedPush {
    #[prost(string, tag = "1")]
    pub display_watching_count: String,
    #[prost(string, tag = "2")]
    pub display_like_count: String,
    #[prost(uint64, tag = "3")]
    pub pending_like_count: u64,
    #[prost(uint64, tag = "4")]
    pub push_interval: u64,
    #[prost(message, repeated, tag = "5")]
    pub comment_feeds: Vec<WebCommentFeed>,
    #[prost(string, tag = "6")]
    pub comment_cursor: String,
    #[prost(message, repeated, tag = "7")]
    pub combo_comment_feed: Vec<WebComboCommentFeed>,
    #[prost(message, repeated, tag = "8")]
    pub like_feeds: Vec<WebLikeFeed>,
    #[prost(message, repeated, tag = "9")]
    pub gift_feeds: Vec<WebGiftFeed>,
    #[prost(string, tag = "10")]
    pub gift_cursor: String,
    #[prost(message, repeated, tag = "11")]
    pub system_notice_feeds: Vec<WebSystemNoticeFeed>,
    #[prost(message, repeated, tag = "12")]
    pub share_feeds: Vec<WebShareFeed>,
}

#[derive(Clone, PartialEq, Message)]
pub struct WebCommentFeed {
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(message, optional, tag = "2")]
    pub user: Option<SimpleUserInfo>,
    #[prost(string, tag = "3")]
    pub content: String,
    #[prost(string, tag = "6")]
    pub color: String,
    #[prost(uint64, tag = "9")]
    pub time: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct SimpleUserInfo {
    #[prost(string, tag = "1")]
    pub principal_id: String,
    #[prost(string, tag = "2")]
    pub user_name: String,
    #[prost(string, tag = "3")]
    pub head_url: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct WebGiftFeed {}

#[derive(Clone, PartialEq, Message)]
pub struct WebLikeFeed {}

#[derive(Clone, PartialEq, Message)]
pub struct WebComboCommentFeed {}

#[derive(Clone, PartialEq, Message)]
pub struct WebSystemNoticeFeed {}

#[derive(Clone, PartialEq, Message)]
pub struct WebShareFeed {}
