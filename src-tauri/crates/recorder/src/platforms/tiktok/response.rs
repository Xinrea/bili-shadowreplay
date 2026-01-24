use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Response structure for TikTok SIGI_STATE data
/// This is a simplified structure as TikTok's actual data structure is very complex
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SigiStateResponse {
    /// LiveRoom data
    #[serde(rename = "LiveRoom")]
    pub live_room: Option<LiveRoom>,

    /// RoomStore data
    #[serde(rename = "RoomStore")]
    pub room_store: Option<RoomStore>,

    /// UserModule data
    #[serde(rename = "UserModule")]
    pub user_module: Option<Value>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomStore {
    pub room_info: Option<RoomInfo>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomInfo {
    #[serde(
        default,
        alias = "room_id",
        alias = "liveRoomId",
        alias = "live_room_id"
    )]
    pub room_id: Option<String>,
    pub title: Option<String>,
    #[serde(default, alias = "liveStatus", alias = "live_status")]
    pub status: Option<i32>,
    #[serde(
        default,
        alias = "ownerInfo",
        alias = "user",
        alias = "userInfo",
        alias = "host",
        alias = "author"
    )]
    pub owner: Option<Owner>,
    #[serde(
        default,
        alias = "streamUrl",
        alias = "stream_url",
        alias = "streamUrlInfo",
        alias = "stream_url_info"
    )]
    pub stream_url: Option<StreamUrl>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Owner {
    #[serde(default, alias = "userId", alias = "user_id")]
    pub id: Option<String>,
    #[serde(default, alias = "uniqueId", alias = "unique_id")]
    pub unique_id: Option<String>,
    #[serde(default, alias = "nickName", alias = "userName", alias = "user_name")]
    pub nickname: Option<String>,
    pub avatar_thumb: Option<Value>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamUrl {
    #[serde(
        default,
        alias = "rtmpPullUrl",
        alias = "rtmp_pull_url",
        alias = "rtmpPlayUrl"
    )]
    pub rtmp_pull_url: Option<String>,
    #[serde(
        default,
        alias = "hlsPullUrl",
        alias = "hls_pull_url",
        alias = "hlsPlayUrl"
    )]
    pub hls_pull_url: Option<String>,
    pub flv_pull_url: Option<Value>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveRoom {
    pub loading_state: LoadingState,
    pub need_login: bool,
    pub show_live_gate: bool,
    pub is_age_gate_room: bool,
    pub recommend_live_rooms: Vec<Value>,
    pub live_room_status: i64,
    pub live_room_user_info: LiveRoomUserInfo,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadingState {
    pub get_recommend_live: i64,
    pub get_user_info: i64,
    pub get_user_stat: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveRoomUserInfo {
    pub user: User,
    pub stats: Stats,
    pub live_room: LiveRoom2,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub avatar_larger: String,
    pub avatar_medium: String,
    pub avatar_thumb: String,
    pub id: String,
    pub nickname: String,
    pub sec_uid: String,
    pub secret: bool,
    pub unique_id: String,
    pub verified: bool,
    pub room_id: String,
    pub signature: String,
    pub status: i64,
    pub follow_status: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub following_count: i64,
    pub follower_count: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveRoom2 {
    pub cover_url: String,
    pub square_cover_img: String,
    pub title: String,
    pub start_time: i64,
    pub status: i64,
    pub paid_event: PaidEvent,
    pub live_sub_only: i64,
    pub live_room_mode: i64,
    pub hash_tag_id: i64,
    pub game_tag_id: i64,
    pub live_room_stats: LiveRoomStats,
    pub stream_data: StreamData,
    pub stream_id: String,
    pub multi_stream_scene: i64,
    pub multi_stream_source: i64,
    pub hevc_stream_data: HevcStreamData,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaidEvent {
    #[serde(rename = "event_id")]
    pub event_id: i64,
    #[serde(rename = "paid_type")]
    pub paid_type: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveRoomStats {
    pub enter_count: i64,
    pub user_count: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamData {
    #[serde(rename = "pull_data")]
    pub pull_data: PullData,
    #[serde(rename = "push_data")]
    pub push_data: PushData,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullData {
    pub options: Options,
    #[serde(rename = "stream_data")]
    pub stream_data: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    #[serde(rename = "default_preview_quality")]
    pub default_preview_quality: DefaultPreviewQuality,
    #[serde(rename = "default_quality")]
    pub default_quality: DefaultQuality,
    pub qualities: Vec<Quality>,
    #[serde(rename = "show_quality_button")]
    pub show_quality_button: bool,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultPreviewQuality {
    #[serde(rename = "icon_type")]
    pub icon_type: i64,
    pub level: i64,
    pub name: String,
    pub resolution: String,
    #[serde(rename = "sdk_key")]
    pub sdk_key: String,
    #[serde(rename = "v_codec")]
    pub v_codec: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultQuality {
    #[serde(rename = "icon_type")]
    pub icon_type: i64,
    pub level: i64,
    pub name: String,
    pub resolution: String,
    #[serde(rename = "sdk_key")]
    pub sdk_key: String,
    #[serde(rename = "v_codec")]
    pub v_codec: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quality {
    #[serde(rename = "icon_type")]
    pub icon_type: i64,
    pub level: i64,
    pub name: String,
    pub resolution: String,
    #[serde(rename = "sdk_key")]
    pub sdk_key: String,
    #[serde(rename = "v_codec")]
    pub v_codec: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushData {
    #[serde(rename = "push_stream_level")]
    pub push_stream_level: i64,
    #[serde(rename = "resolution_params")]
    pub resolution_params: ResolutionParams,
    #[serde(rename = "stream_data")]
    pub stream_data: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionParams {}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HevcStreamData {
    #[serde(rename = "pull_data")]
    pub pull_data: PullData2,
    #[serde(rename = "push_data")]
    pub push_data: PushData2,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullData2 {
    pub options: Options2,
    #[serde(rename = "stream_data")]
    pub stream_data: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options2 {
    #[serde(rename = "default_preview_quality")]
    pub default_preview_quality: DefaultPreviewQuality2,
    #[serde(rename = "default_quality")]
    pub default_quality: DefaultQuality2,
    pub qualities: Vec<Quality2>,
    #[serde(rename = "show_quality_button")]
    pub show_quality_button: bool,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultPreviewQuality2 {
    #[serde(rename = "icon_type")]
    pub icon_type: i64,
    pub level: i64,
    pub name: String,
    pub resolution: String,
    #[serde(rename = "sdk_key")]
    pub sdk_key: String,
    #[serde(rename = "v_codec")]
    pub v_codec: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultQuality2 {
    #[serde(rename = "icon_type")]
    pub icon_type: i64,
    pub level: i64,
    pub name: String,
    pub resolution: String,
    #[serde(rename = "sdk_key")]
    pub sdk_key: String,
    #[serde(rename = "v_codec")]
    pub v_codec: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quality2 {
    #[serde(rename = "icon_type")]
    pub icon_type: i64,
    pub level: i64,
    pub name: String,
    pub resolution: String,
    #[serde(rename = "sdk_key")]
    pub sdk_key: String,
    #[serde(rename = "v_codec")]
    pub v_codec: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushData2 {
    #[serde(rename = "push_stream_level")]
    pub push_stream_level: i64,
    #[serde(rename = "resolution_params")]
    pub resolution_params: ResolutionParams2,
    #[serde(rename = "stream_data")]
    pub stream_data: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionParams2 {}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentRoom {
    pub loading_state: LoadingState2,
    pub room_info: Value,
    pub anchor_id: String,
    pub sec_anchor_id: String,
    pub anchor_unique_id: String,
    pub room_id: String,
    pub hot_live_room_info: Value,
    pub live_type: String,
    pub report_link_type: String,
    #[serde(rename = "enterRoomWithSSR")]
    pub enter_room_with_ssr: bool,
    pub play_mode: String,
    pub is_guest_connection: bool,
    pub is_multi_guest_room: bool,
    pub show_live_chat: bool,
    pub enable_chat: bool,
    pub is_answer_room: bool,
    pub is_gate_room: bool,
    pub request_id: String,
    pub ntp_diff: i64,
    pub follow_status_map: FollowStatusMap,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadingState2 {
    pub enter_room: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowStatusMap {}
