use serde_derive::Deserialize;
use serde_derive::Serialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DouyinRoomInfoResponse {
    pub data: Data,
    #[serde(default)]
    pub extra: Option<serde_json::Value>,
    #[serde(rename = "status_code")]
    pub status_code: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Data {
    pub data: Vec<Daum>,
    #[serde(rename = "enter_room_id", default)]
    pub enter_room_id: Option<String>,
    #[serde(default)]
    pub extra: Option<serde_json::Value>,
    pub user: User,
    #[serde(rename = "qrcode_url", default)]
    pub qrcode_url: Option<String>,
    #[serde(rename = "enter_mode", default)]
    pub enter_mode: Option<i64>,
    #[serde(rename = "room_status")]
    pub room_status: i64,
    #[serde(rename = "partition_road_map", default)]
    pub partition_road_map: Option<serde_json::Value>,
    #[serde(rename = "similar_rooms", default)]
    pub similar_rooms: Option<Vec<serde_json::Value>>,
    #[serde(rename = "shark_decision_conf", default)]
    pub shark_decision_conf: Option<String>,
    #[serde(rename = "web_stream_url", default)]
    pub web_stream_url: Option<serde_json::Value>,
    #[serde(rename = "login_lead", default)]
    pub login_lead: Option<serde_json::Value>,
    #[serde(rename = "auth_cert_info", default)]
    pub auth_cert_info: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Daum {
    #[serde(rename = "id_str")]
    pub id_str: String,
    pub status: i64,
    #[serde(rename = "status_str")]
    pub status_str: String,
    pub title: String,
    #[serde(rename = "user_count_str", default)]
    pub user_count_str: Option<String>,
    pub cover: Option<Cover>,
    #[serde(rename = "stream_url")]
    pub stream_url: Option<StreamUrl>,
    #[serde(default)]
    pub owner: Option<Owner>,
    #[serde(rename = "room_auth", default)]
    pub room_auth: Option<RoomAuth>,
    #[serde(rename = "live_room_mode", default)]
    pub live_room_mode: Option<i64>,
    #[serde(default)]
    pub stats: Option<Stats>,
    #[serde(rename = "has_commerce_goods", default)]
    pub has_commerce_goods: Option<bool>,
    #[serde(rename = "linker_map", default)]
    pub linker_map: Option<LinkerMap>,
    #[serde(rename = "linker_detail", default)]
    pub linker_detail: Option<LinkerDetail>,
    #[serde(rename = "room_view_stats", default)]
    pub room_view_stats: Option<RoomViewStats>,
    #[serde(rename = "scene_type_info", default)]
    pub scene_type_info: Option<SceneTypeInfo>,
    #[serde(rename = "like_count", default)]
    pub like_count: Option<i64>,
    #[serde(rename = "owner_user_id_str", default)]
    pub owner_user_id_str: Option<String>,
    // Many other fields that can be ignored for now
    #[serde(flatten)]
    pub other_fields: HashMap<String, serde_json::Value>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cover {
    #[serde(rename = "url_list")]
    pub url_list: Vec<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamUrl {
    #[serde(rename = "flv_pull_url")]
    pub flv_pull_url: FlvPullUrl,
    #[serde(rename = "default_resolution")]
    pub default_resolution: String,
    #[serde(rename = "hls_pull_url_map")]
    pub hls_pull_url_map: HlsPullUrlMap,
    #[serde(rename = "hls_pull_url")]
    pub hls_pull_url: String,
    #[serde(rename = "stream_orientation")]
    pub stream_orientation: i64,
    #[serde(rename = "live_core_sdk_data")]
    pub live_core_sdk_data: LiveCoreSdkData,
    pub extra: Extra,
    #[serde(rename = "pull_datas", default)]
    pub pull_datas: Option<serde_json::Value>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlvPullUrl {
    #[serde(rename = "FULL_HD1")]
    pub full_hd1: Option<String>,
    #[serde(rename = "HD1")]
    pub hd1: Option<String>,
    #[serde(rename = "SD1")]
    pub sd1: Option<String>,
    #[serde(rename = "SD2")]
    pub sd2: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HlsPullUrlMap {
    #[serde(rename = "FULL_HD1")]
    pub full_hd1: Option<String>,
    #[serde(rename = "HD1")]
    pub hd1: Option<String>,
    #[serde(rename = "SD1")]
    pub sd1: Option<String>,
    #[serde(rename = "SD2")]
    pub sd2: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveCoreSdkData {
    #[serde(rename = "pull_data")]
    pub pull_data: PullData,
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
    #[serde(rename = "default_quality")]
    pub default_quality: DefaultQuality,
    pub qualities: Vec<Quality>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultQuality {
    pub name: String,
    #[serde(rename = "sdk_key")]
    pub sdk_key: String,
    #[serde(rename = "v_codec")]
    pub v_codec: String,
    pub resolution: String,
    pub level: i64,
    #[serde(rename = "v_bit_rate")]
    pub v_bit_rate: i64,
    #[serde(rename = "additional_content")]
    pub additional_content: String,
    pub fps: i64,
    pub disable: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quality {
    pub name: String,
    #[serde(rename = "sdk_key")]
    pub sdk_key: String,
    #[serde(rename = "v_codec")]
    pub v_codec: String,
    pub resolution: String,
    pub level: i64,
    #[serde(rename = "v_bit_rate")]
    pub v_bit_rate: i64,
    #[serde(rename = "additional_content")]
    pub additional_content: String,
    pub fps: i64,
    pub disable: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Extra {
    pub height: i64,
    pub width: i64,
    pub fps: i64,
    #[serde(rename = "max_bitrate")]
    pub max_bitrate: i64,
    #[serde(rename = "min_bitrate")]
    pub min_bitrate: i64,
    #[serde(rename = "default_bitrate")]
    pub default_bitrate: i64,
    #[serde(rename = "bitrate_adapt_strategy")]
    pub bitrate_adapt_strategy: i64,
    #[serde(rename = "anchor_interact_profile")]
    pub anchor_interact_profile: i64,
    #[serde(rename = "audience_interact_profile")]
    pub audience_interact_profile: i64,
    #[serde(rename = "hardware_encode")]
    pub hardware_encode: bool,
    #[serde(rename = "video_profile")]
    pub video_profile: i64,
    #[serde(rename = "h265_enable")]
    pub h265_enable: bool,
    #[serde(rename = "gop_sec")]
    pub gop_sec: i64,
    #[serde(rename = "bframe_enable")]
    pub bframe_enable: bool,
    pub roi: bool,
    #[serde(rename = "sw_roi")]
    pub sw_roi: bool,
    #[serde(rename = "bytevc1_enable")]
    pub bytevc1_enable: bool,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct Owner {
    #[serde(rename = "id_str")]
    pub id_str: String,
    #[serde(rename = "sec_uid")]
    pub sec_uid: String,
    pub nickname: String,
    #[serde(rename = "avatar_thumb")]
    pub avatar_thumb: AvatarThumb,
    #[serde(rename = "follow_info")]
    pub follow_info: FollowInfo,
    pub subscribe: Subscribe,
    #[serde(rename = "foreign_user")]
    pub foreign_user: i64,
    #[serde(rename = "open_id_str")]
    pub open_id_str: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarThumb {
    #[serde(rename = "url_list")]
    pub url_list: Vec<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowInfo {
    #[serde(rename = "follow_status")]
    pub follow_status: i64,
    #[serde(rename = "follow_status_str")]
    pub follow_status_str: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subscribe {
    #[serde(rename = "is_member")]
    pub is_member: bool,
    pub level: i64,
    #[serde(rename = "identity_type")]
    pub identity_type: i64,
    #[serde(rename = "buy_type")]
    pub buy_type: i64,
    pub open: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct RoomAuth {
    #[serde(rename = "Chat")]
    pub chat: bool,
    #[serde(rename = "Danmaku")]
    pub danmaku: bool,
    #[serde(rename = "Gift")]
    pub gift: bool,
    #[serde(rename = "LuckMoney")]
    pub luck_money: bool,
    #[serde(rename = "Digg")]
    pub digg: bool,
    #[serde(rename = "RoomContributor")]
    pub room_contributor: bool,
    #[serde(rename = "Props")]
    pub props: bool,
    #[serde(rename = "UserCard")]
    pub user_card: bool,
    #[serde(rename = "POI")]
    pub poi: bool,
    #[serde(rename = "MoreAnchor")]
    pub more_anchor: i64,
    #[serde(rename = "Banner")]
    pub banner: i64,
    #[serde(rename = "Share")]
    pub share: i64,
    #[serde(rename = "UserCorner")]
    pub user_corner: i64,
    #[serde(rename = "Landscape")]
    pub landscape: i64,
    #[serde(rename = "LandscapeChat")]
    pub landscape_chat: i64,
    #[serde(rename = "PublicScreen")]
    pub public_screen: i64,
    #[serde(rename = "GiftAnchorMt")]
    pub gift_anchor_mt: i64,
    #[serde(rename = "RecordScreen")]
    pub record_screen: i64,
    #[serde(rename = "DonationSticker")]
    pub donation_sticker: i64,
    #[serde(rename = "HourRank")]
    pub hour_rank: i64,
    #[serde(rename = "CommerceCard")]
    pub commerce_card: i64,
    #[serde(rename = "AudioChat")]
    pub audio_chat: i64,
    #[serde(rename = "DanmakuDefault")]
    pub danmaku_default: i64,
    #[serde(rename = "KtvOrderSong")]
    pub ktv_order_song: i64,
    #[serde(rename = "SelectionAlbum")]
    pub selection_album: i64,
    #[serde(rename = "Like")]
    pub like: i64,
    #[serde(rename = "MultiplierPlayback")]
    pub multiplier_playback: i64,
    #[serde(rename = "DownloadVideo")]
    pub download_video: i64,
    #[serde(rename = "Collect")]
    pub collect: i64,
    #[serde(rename = "TimedShutdown")]
    pub timed_shutdown: i64,
    #[serde(rename = "Seek")]
    pub seek: i64,
    #[serde(rename = "Denounce")]
    pub denounce: i64,
    #[serde(rename = "Dislike")]
    pub dislike: i64,
    #[serde(rename = "OnlyTa")]
    pub only_ta: i64,
    #[serde(rename = "CastScreen")]
    pub cast_screen: i64,
    #[serde(rename = "CommentWall")]
    pub comment_wall: i64,
    #[serde(rename = "BulletStyle")]
    pub bullet_style: i64,
    #[serde(rename = "ShowGamePlugin")]
    pub show_game_plugin: i64,
    #[serde(rename = "VSGift")]
    pub vsgift: i64,
    #[serde(rename = "VSTopic")]
    pub vstopic: i64,
    #[serde(rename = "VSRank")]
    pub vsrank: i64,
    #[serde(rename = "AdminCommentWall")]
    pub admin_comment_wall: i64,
    #[serde(rename = "CommerceComponent")]
    pub commerce_component: i64,
    #[serde(rename = "DouPlus")]
    pub dou_plus: i64,
    #[serde(rename = "GamePointsPlaying")]
    pub game_points_playing: i64,
    #[serde(rename = "Poster")]
    pub poster: i64,
    #[serde(rename = "Highlights")]
    pub highlights: i64,
    #[serde(rename = "TypingCommentState")]
    pub typing_comment_state: i64,
    #[serde(rename = "StrokeUpDownGuide")]
    pub stroke_up_down_guide: i64,
    #[serde(rename = "UpRightStatsFloatingLayer")]
    pub up_right_stats_floating_layer: i64,
    #[serde(rename = "CastScreenExplicit")]
    pub cast_screen_explicit: i64,
    #[serde(rename = "Selection")]
    pub selection: i64,
    #[serde(rename = "IndustryService")]
    pub industry_service: i64,
    #[serde(rename = "VerticalRank")]
    pub vertical_rank: i64,
    #[serde(rename = "EnterEffects")]
    pub enter_effects: i64,
    #[serde(rename = "FansClub")]
    pub fans_club: i64,
    #[serde(rename = "EmojiOutside")]
    pub emoji_outside: i64,
    #[serde(rename = "CanSellTicket")]
    pub can_sell_ticket: i64,
    #[serde(rename = "DouPlusPopularityGem")]
    pub dou_plus_popularity_gem: i64,
    #[serde(rename = "MissionCenter")]
    pub mission_center: i64,
    #[serde(rename = "ExpandScreen")]
    pub expand_screen: i64,
    #[serde(rename = "FansGroup")]
    pub fans_group: i64,
    #[serde(rename = "Topic")]
    pub topic: i64,
    #[serde(rename = "AnchorMission")]
    pub anchor_mission: i64,
    #[serde(rename = "Teleprompter")]
    pub teleprompter: i64,
    #[serde(rename = "LongTouch")]
    pub long_touch: i64,
    #[serde(rename = "FirstFeedHistChat")]
    pub first_feed_hist_chat: i64,
    #[serde(rename = "MoreHistChat")]
    pub more_hist_chat: i64,
    #[serde(rename = "TaskBanner")]
    pub task_banner: i64,
    #[serde(rename = "SpecialStyle")]
    pub special_style: SpecialStyle,
    #[serde(rename = "FixedChat")]
    pub fixed_chat: i64,
    #[serde(rename = "QuizGamePointsPlaying")]
    pub quiz_game_points_playing: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct SpecialStyle {
    #[serde(rename = "Chat")]
    pub chat: Chat,
    #[serde(rename = "Like")]
    pub like: Like,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct Chat {
    #[serde(rename = "UnableStyle")]
    pub unable_style: i64,
    #[serde(rename = "Content")]
    pub content: String,
    #[serde(rename = "OffType")]
    pub off_type: i64,
    #[serde(rename = "AnchorSwitchForPaidLive")]
    pub anchor_switch_for_paid_live: i64,
    #[serde(rename = "ContentForPaidLive")]
    pub content_for_paid_live: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct Like {
    #[serde(rename = "UnableStyle")]
    pub unable_style: i64,
    #[serde(rename = "Content")]
    pub content: String,
    #[serde(rename = "OffType")]
    pub off_type: i64,
    #[serde(rename = "AnchorSwitchForPaidLive")]
    pub anchor_switch_for_paid_live: i64,
    #[serde(rename = "ContentForPaidLive")]
    pub content_for_paid_live: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct Stats {
    #[serde(rename = "total_user_desp")]
    pub total_user_desp: String,
    #[serde(rename = "like_count")]
    pub like_count: i64,
    #[serde(rename = "total_user_str")]
    pub total_user_str: String,
    #[serde(rename = "user_count_str")]
    pub user_count_str: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct LinkerMap {}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct LinkerDetail {
    #[serde(rename = "linker_play_modes")]
    pub linker_play_modes: Vec<Value>,
    #[serde(rename = "big_party_layout_config_version")]
    pub big_party_layout_config_version: i64,
    #[serde(rename = "accept_audience_pre_apply")]
    pub accept_audience_pre_apply: bool,
    #[serde(rename = "linker_ui_layout")]
    pub linker_ui_layout: i64,
    #[serde(rename = "enable_audience_linkmic")]
    pub enable_audience_linkmic: i64,
    #[serde(rename = "function_type")]
    pub function_type: String,
    #[serde(rename = "linker_map_str")]
    pub linker_map_str: LinkerMapStr,
    #[serde(rename = "ktv_lyric_mode")]
    pub ktv_lyric_mode: String,
    #[serde(rename = "init_source")]
    pub init_source: String,
    #[serde(rename = "forbid_apply_from_other")]
    pub forbid_apply_from_other: bool,
    #[serde(rename = "ktv_exhibit_mode")]
    pub ktv_exhibit_mode: i64,
    #[serde(rename = "enlarge_guest_turn_on_source")]
    pub enlarge_guest_turn_on_source: i64,
    #[serde(rename = "playmode_detail")]
    pub playmode_detail: PlaymodeDetail,
    #[serde(rename = "client_ui_info")]
    pub client_ui_info: String,
    #[serde(rename = "manual_open_ui")]
    pub manual_open_ui: i64,
    #[serde(rename = "feature_list")]
    pub feature_list: Vec<Value>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct LinkerMapStr {}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct PlaymodeDetail {}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct RoomViewStats {
    #[serde(rename = "is_hidden")]
    pub is_hidden: bool,
    #[serde(rename = "display_short")]
    pub display_short: String,
    #[serde(rename = "display_middle")]
    pub display_middle: String,
    #[serde(rename = "display_long")]
    pub display_long: String,
    #[serde(rename = "display_value")]
    pub display_value: i64,
    #[serde(rename = "display_version")]
    pub display_version: i64,
    pub incremental: bool,
    #[serde(rename = "display_type")]
    pub display_type: i64,
    #[serde(rename = "display_short_anchor")]
    pub display_short_anchor: String,
    #[serde(rename = "display_middle_anchor")]
    pub display_middle_anchor: String,
    #[serde(rename = "display_long_anchor")]
    pub display_long_anchor: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct SceneTypeInfo {
    #[serde(rename = "is_union_live_room")]
    pub is_union_live_room: bool,
    #[serde(rename = "is_life")]
    pub is_life: bool,
    #[serde(rename = "is_protected_room")]
    pub is_protected_room: i64,
    #[serde(rename = "is_lasted_goods_room")]
    pub is_lasted_goods_room: i64,
    #[serde(rename = "is_desire_room")]
    pub is_desire_room: i64,
    #[serde(rename = "commentary_type")]
    pub commentary_type: bool,
    #[serde(rename = "is_sub_orientation_vertical_room")]
    pub is_sub_orientation_vertical_room: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct EntranceList {
    #[serde(rename = "group_id")]
    pub group_id: i64,
    #[serde(rename = "component_type")]
    pub component_type: i64,
    #[serde(rename = "op_type")]
    pub op_type: i64,
    pub text: String,
    #[serde(rename = "schema_url")]
    pub schema_url: String,
    #[serde(rename = "show_type")]
    pub show_type: i64,
    #[serde(rename = "data_status")]
    pub data_status: i64,
    pub extra: String,
    pub icon: Option<Icon>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct Icon {
    #[serde(rename = "url_list")]
    pub url_list: Vec<String>,
    pub uri: String,
    pub height: i64,
    pub width: i64,
    #[serde(rename = "avg_color")]
    pub avg_color: String,
    #[serde(rename = "image_type")]
    pub image_type: i64,
    #[serde(rename = "open_web_url")]
    pub open_web_url: String,
    #[serde(rename = "is_animated")]
    pub is_animated: bool,
    #[serde(rename = "flex_setting_list")]
    pub flex_setting_list: Vec<Value>,
    #[serde(rename = "text_setting_list")]
    pub text_setting_list: Vec<Value>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    #[serde(rename = "id_str")]
    pub id_str: String,
    #[serde(rename = "sec_uid")]
    pub sec_uid: String,
    pub nickname: String,
    #[serde(rename = "avatar_thumb")]
    pub avatar_thumb: AvatarThumb,
    #[serde(rename = "follow_info")]
    pub follow_info: FollowInfo,
    #[serde(rename = "foreign_user")]
    pub foreign_user: i64,
    #[serde(rename = "open_id_str")]
    pub open_id_str: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DouyinRelationResponse {
    pub extra: Option<Extra2>,
    pub followings: Option<Vec<Following>>,
    #[serde(rename = "owner_sec_uid")]
    pub owner_sec_uid: String,
    #[serde(rename = "status_code")]
    pub status_code: i64,
    #[serde(rename = "log_pb")]
    pub log_pb: Option<LogPb>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Extra2 {
    #[serde(rename = "fatal_item_ids")]
    pub fatal_item_ids: Vec<String>,
    pub logid: String,
    pub now: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogPb {
    #[serde(rename = "impr_id")]
    pub impr_id: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Following {
    #[serde(rename = "account_cert_info")]
    pub account_cert_info: String,
    #[serde(rename = "avatar_signature")]
    pub avatar_signature: String,
    #[serde(rename = "avatar_small")]
    pub avatar_small: AvatarSmall,
    #[serde(rename = "avatar_thumb")]
    pub avatar_thumb: AvatarThumb,
    #[serde(rename = "birthday_hide_level")]
    pub birthday_hide_level: i64,
    #[serde(rename = "commerce_user_level")]
    pub commerce_user_level: i64,
    #[serde(rename = "custom_verify")]
    pub custom_verify: String,
    #[serde(rename = "enterprise_verify_reason")]
    pub enterprise_verify_reason: String,
    #[serde(rename = "follow_status")]
    pub follow_status: i64,
    #[serde(rename = "follower_status")]
    pub follower_status: i64,
    #[serde(rename = "has_e_account_role")]
    pub has_e_account_role: bool,
    #[serde(rename = "im_activeness")]
    pub im_activeness: i64,
    #[serde(rename = "im_role_ids")]
    pub im_role_ids: Vec<serde_json::Value>,
    #[serde(rename = "is_im_oversea_user")]
    pub is_im_oversea_user: i64,
    pub nickname: String,
    #[serde(rename = "sec_uid")]
    pub sec_uid: String,
    #[serde(rename = "short_id")]
    pub short_id: String,
    pub signature: String,
    #[serde(rename = "social_relation_sub_type")]
    pub social_relation_sub_type: i64,
    #[serde(rename = "social_relation_type")]
    pub social_relation_type: i64,
    pub uid: String,
    #[serde(rename = "unique_id")]
    pub unique_id: String,
    #[serde(rename = "verification_type")]
    pub verification_type: i64,
    #[serde(rename = "webcast_sp_info")]
    pub webcast_sp_info: serde_json::Value,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvatarSmall {
    pub uri: String,
    #[serde(rename = "url_list")]
    pub url_list: Vec<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DouyinH5RoomInfoResponse {
    pub data: H5Data,
    pub extra: H5Extra,
    #[serde(rename = "status_code")]
    pub status_code: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct H5Data {
    pub room: H5Room,
    pub user: H5User,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct H5Room {
    pub id: u64,
    #[serde(rename = "id_str")]
    pub id_str: String,
    pub status: i64,
    pub title: String,
    pub cover: Option<H5Cover>,
    #[serde(rename = "stream_url")]
    pub stream_url: Option<H5StreamUrl>,
    pub owner: H5Owner,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct H5Cover {
    #[serde(rename = "url_list")]
    pub url_list: Vec<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct H5StreamUrl {
    pub provider: i64,
    pub id: u64,
    #[serde(rename = "id_str")]
    pub id_str: String,
    #[serde(rename = "default_resolution")]
    pub default_resolution: String,
    #[serde(rename = "rtmp_pull_url")]
    pub rtmp_pull_url: String,
    #[serde(rename = "flv_pull_url")]
    pub flv_pull_url: H5FlvPullUrl,
    #[serde(rename = "hls_pull_url")]
    pub hls_pull_url: String,
    #[serde(rename = "hls_pull_url_map")]
    pub hls_pull_url_map: H5HlsPullUrlMap,
    #[serde(rename = "live_core_sdk_data")]
    pub live_core_sdk_data: LiveCoreSdkData,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct H5FlvPullUrl {
    #[serde(rename = "FULL_HD1")]
    pub full_hd1: Option<String>,
    #[serde(rename = "HD1")]
    pub hd1: Option<String>,
    #[serde(rename = "SD1")]
    pub sd1: Option<String>,
    #[serde(rename = "SD2")]
    pub sd2: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct H5HlsPullUrlMap {
    #[serde(rename = "FULL_HD1")]
    pub full_hd1: Option<String>,
    #[serde(rename = "HD1")]
    pub hd1: Option<String>,
    #[serde(rename = "SD1")]
    pub sd1: Option<String>,
    #[serde(rename = "SD2")]
    pub sd2: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct H5Owner {
    pub nickname: String,
    #[serde(rename = "avatar_thumb")]
    pub avatar_thumb: H5AvatarThumb,
    #[serde(rename = "sec_uid")]
    pub sec_uid: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct H5AvatarThumb {
    #[serde(rename = "url_list")]
    pub url_list: Vec<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct H5User {
    pub nickname: String,
    #[serde(rename = "avatar_thumb")]
    pub avatar_thumb: Option<H5AvatarThumb>,
    #[serde(rename = "sec_uid")]
    pub sec_uid: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct H5Extra {
    pub now: i64,
}
