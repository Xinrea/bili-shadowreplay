use prost::Message;
use std::collections::HashMap;

// message Response {
//   repeated Message messagesList = 1;
//   string cursor = 2;
//   uint64 fetchInterval = 3;
//   uint64 now = 4;
//   string internalExt = 5;
//   uint32 fetchType = 6;
//   map<string, string> routeParams = 7;
//   uint64 heartbeatDuration = 8;
//   bool needAck = 9;
//   string pushServer = 10;
//   string liveCursor = 11;
//   bool historyNoMore = 12;
// }

#[derive(Message)]
pub struct Response {
    #[prost(message, repeated, tag = "1")]
    pub messages_list: Vec<CommonMessage>,
    #[prost(string, tag = "2")]
    pub cursor: String,
    #[prost(uint64, tag = "3")]
    pub fetch_interval: u64,
    #[prost(uint64, tag = "4")]
    pub now: u64,
    #[prost(string, tag = "5")]
    pub internal_ext: String,
    #[prost(uint32, tag = "6")]
    pub fetch_type: u32,
    #[prost(map = "string, string", tag = "7")]
    pub route_params: HashMap<String, String>,
    #[prost(uint64, tag = "8")]
    pub heartbeat_duration: u64,
    #[prost(bool, tag = "9")]
    pub need_ack: bool,
    #[prost(string, tag = "10")]
    pub push_server: String,
    #[prost(string, tag = "11")]
    pub live_cursor: String,
    #[prost(bool, tag = "12")]
    pub history_no_more: bool,
}

#[derive(Message)]
pub struct CommonMessage {
    #[prost(string, tag = "1")]
    pub method: String,
    #[prost(bytes, tag = "2")]
    pub payload: Vec<u8>,
    #[prost(int64, tag = "3")]
    pub msg_id: i64,
    #[prost(int32, tag = "4")]
    pub msg_type: i32,
    #[prost(int64, tag = "5")]
    pub offset: i64,
    #[prost(bool, tag = "6")]
    pub need_wrds_store: bool,
    #[prost(int64, tag = "7")]
    pub wrds_version: i64,
    #[prost(string, tag = "8")]
    pub wrds_sub_key: String,
}

#[derive(Message)]
pub struct Common {
    #[prost(string, tag = "1")]
    pub method: String,
    #[prost(uint64, tag = "2")]
    pub msg_id: u64,
    #[prost(uint64, tag = "3")]
    pub room_id: u64,
    #[prost(uint64, tag = "4")]
    pub create_time: u64,
    #[prost(uint32, tag = "5")]
    pub monitor: u32,
    #[prost(bool, tag = "6")]
    pub is_show_msg: bool,
    #[prost(string, tag = "7")]
    pub describe: String,
    #[prost(uint64, tag = "9")]
    pub fold_type: u64,
    #[prost(uint64, tag = "10")]
    pub anchor_fold_type: u64,
    #[prost(uint64, tag = "11")]
    pub priority_score: u64,
    #[prost(string, tag = "12")]
    pub log_id: String,
    #[prost(string, tag = "13")]
    pub msg_process_filter_k: String,
    #[prost(string, tag = "14")]
    pub msg_process_filter_v: String,
    #[prost(message, optional, tag = "15")]
    pub user: Option<User>,
}

#[derive(Message)]
pub struct User {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(uint64, tag = "2")]
    pub short_id: u64,
    #[prost(string, tag = "3")]
    pub nick_name: String,
    #[prost(uint32, tag = "4")]
    pub gender: u32,
    #[prost(string, tag = "5")]
    pub signature: String,
    #[prost(uint32, tag = "6")]
    pub level: u32,
    #[prost(uint64, tag = "7")]
    pub birthday: u64,
    #[prost(string, tag = "8")]
    pub telephone: String,
    #[prost(message, optional, tag = "9")]
    pub avatar_thumb: Option<Image>,
    #[prost(message, optional, tag = "10")]
    pub avatar_medium: Option<Image>,
    #[prost(message, optional, tag = "11")]
    pub avatar_large: Option<Image>,
    #[prost(bool, tag = "12")]
    pub verified: bool,
    #[prost(uint32, tag = "13")]
    pub experience: u32,
    #[prost(string, tag = "14")]
    pub city: String,
    #[prost(int32, tag = "15")]
    pub status: i32,
    #[prost(uint64, tag = "16")]
    pub create_time: u64,
    #[prost(uint64, tag = "17")]
    pub modify_time: u64,
    #[prost(uint32, tag = "18")]
    pub secret: u32,
    #[prost(string, tag = "19")]
    pub share_qrcode_uri: String,
    #[prost(uint32, tag = "20")]
    pub income_share_percent: u32,
    #[prost(message, repeated, tag = "21")]
    pub badge_image_list: Vec<Image>,
    #[prost(message, optional, tag = "22")]
    pub follow_info: Option<FollowInfo>,
    #[prost(message, optional, tag = "23")]
    pub pay_grade: Option<PayGrade>,
    #[prost(message, optional, tag = "24")]
    pub fans_club: Option<FansClub>,
    #[prost(string, tag = "26")]
    pub special_id: String,
    #[prost(message, optional, tag = "27")]
    pub avatar_border: Option<Image>,
    #[prost(message, optional, tag = "28")]
    pub medal: Option<Image>,
    #[prost(message, repeated, tag = "29")]
    pub real_time_icons_list: Vec<Image>,
    #[prost(string, tag = "38")]
    pub display_id: String,
    #[prost(string, tag = "46")]
    pub sec_uid: String,
    #[prost(uint64, tag = "1022")]
    pub fan_ticket_count: u64,
    #[prost(string, tag = "1028")]
    pub id_str: String,
    #[prost(uint32, tag = "1045")]
    pub age_range: u32,
}

#[derive(Message, PartialEq)]
pub struct Image {
    #[prost(string, repeated, tag = "1")]
    pub url_list_list: Vec<String>,
    #[prost(string, tag = "2")]
    pub uri: String,
    #[prost(uint64, tag = "3")]
    pub height: u64,
    #[prost(uint64, tag = "4")]
    pub width: u64,
    #[prost(string, tag = "5")]
    pub avg_color: String,
    #[prost(uint32, tag = "6")]
    pub image_type: u32,
    #[prost(string, tag = "7")]
    pub open_web_url: String,
    #[prost(message, optional, tag = "8")]
    pub content: Option<ImageContent>,
    #[prost(bool, tag = "9")]
    pub is_animated: bool,
    #[prost(message, optional, tag = "10")]
    pub flex_setting_list: Option<NinePatchSetting>,
    #[prost(message, optional, tag = "11")]
    pub text_setting_list: Option<NinePatchSetting>,
}

#[derive(Message, PartialEq)]
pub struct ImageContent {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub font_color: String,
    #[prost(uint64, tag = "3")]
    pub level: u64,
    #[prost(string, tag = "4")]
    pub alternative_text: String,
}

#[derive(Message, PartialEq)]
pub struct NinePatchSetting {
    #[prost(string, repeated, tag = "1")]
    pub setting_list_list: Vec<String>,
}

#[derive(Message)]
pub struct FollowInfo {
    #[prost(uint64, tag = "1")]
    pub following_count: u64,
    #[prost(uint64, tag = "2")]
    pub follower_count: u64,
    #[prost(uint64, tag = "3")]
    pub follow_status: u64,
    #[prost(uint64, tag = "4")]
    pub push_status: u64,
    #[prost(string, tag = "5")]
    pub remark_name: String,
    #[prost(string, tag = "6")]
    pub follower_count_str: String,
    #[prost(string, tag = "7")]
    pub following_count_str: String,
}

#[derive(Message)]
pub struct PayGrade {
    #[prost(int64, tag = "1")]
    pub total_diamond_count: i64,
    #[prost(message, optional, tag = "2")]
    pub diamond_icon: Option<Image>,
    #[prost(string, tag = "3")]
    pub name: String,
    #[prost(message, optional, tag = "4")]
    pub icon: Option<Image>,
    #[prost(string, tag = "5")]
    pub next_name: String,
    #[prost(int64, tag = "6")]
    pub level: i64,
    #[prost(message, optional, tag = "7")]
    pub next_icon: Option<Image>,
    #[prost(int64, tag = "8")]
    pub next_diamond: i64,
    #[prost(int64, tag = "9")]
    pub now_diamond: i64,
    #[prost(int64, tag = "10")]
    pub this_grade_min_diamond: i64,
    #[prost(int64, tag = "11")]
    pub this_grade_max_diamond: i64,
    #[prost(int64, tag = "12")]
    pub pay_diamond_bak: i64,
    #[prost(string, tag = "13")]
    pub grade_describe: String,
    #[prost(message, repeated, tag = "14")]
    pub grade_icon_list: Vec<GradeIcon>,
    #[prost(int64, tag = "15")]
    pub screen_chat_type: i64,
    #[prost(message, optional, tag = "16")]
    pub im_icon: Option<Image>,
    #[prost(message, optional, tag = "17")]
    pub im_icon_with_level: Option<Image>,
    #[prost(message, optional, tag = "18")]
    pub live_icon: Option<Image>,
    #[prost(message, optional, tag = "19")]
    pub new_im_icon_with_level: Option<Image>,
    #[prost(message, optional, tag = "20")]
    pub new_live_icon: Option<Image>,
    #[prost(int64, tag = "21")]
    pub upgrade_need_consume: i64,
    #[prost(string, tag = "22")]
    pub next_privileges: String,
    #[prost(message, optional, tag = "23")]
    pub background: Option<Image>,
    #[prost(message, optional, tag = "24")]
    pub background_back: Option<Image>,
    #[prost(int64, tag = "25")]
    pub score: i64,
    #[prost(message, optional, tag = "26")]
    pub buff_info: Option<GradeBuffInfo>,
}

#[derive(Message)]
pub struct GradeIcon {
    #[prost(message, optional, tag = "1")]
    pub icon: Option<Image>,
    #[prost(int64, tag = "2")]
    pub icon_diamond: i64,
    #[prost(int64, tag = "3")]
    pub level: i64,
    #[prost(string, tag = "4")]
    pub level_str: String,
}

#[derive(Message)]
pub struct GradeBuffInfo {}

#[derive(Message)]
pub struct FansClub {
    #[prost(message, optional, tag = "1")]
    pub data: Option<FansClubData>,
    #[prost(map = "int32, message", tag = "2")]
    pub prefer_data: HashMap<i32, FansClubData>,
}

#[derive(Message, PartialEq)]
pub struct FansClubData {
    #[prost(string, tag = "1")]
    pub club_name: String,
    #[prost(int32, tag = "2")]
    pub level: i32,
    #[prost(int32, tag = "3")]
    pub user_fans_club_status: i32,
    #[prost(message, optional, tag = "4")]
    pub badge: Option<UserBadge>,
    #[prost(int64, repeated, tag = "5")]
    pub available_gift_ids: Vec<i64>,
    #[prost(int64, tag = "6")]
    pub anchor_id: i64,
}

#[derive(Message, PartialEq)]
pub struct UserBadge {
    #[prost(map = "int32, message", tag = "1")]
    pub icons: HashMap<i32, Image>,
    #[prost(string, tag = "2")]
    pub title: String,
}

#[derive(Message)]
pub struct PublicAreaCommon {
    #[prost(message, optional, tag = "1")]
    pub user_label: Option<Image>,
    #[prost(uint64, tag = "2")]
    pub user_consume_in_room: u64,
    #[prost(uint64, tag = "3")]
    pub user_send_gift_cnt_in_room: u64,
}

#[derive(Message)]
pub struct LandscapeAreaCommon {
    #[prost(bool, tag = "1")]
    pub show_head: bool,
    #[prost(bool, tag = "2")]
    pub show_nickname: bool,
    #[prost(bool, tag = "3")]
    pub show_font_color: bool,
    #[prost(string, repeated, tag = "4")]
    pub color_value_list: Vec<String>,
    #[prost(enumeration = "CommentTypeTag", repeated, tag = "5")]
    pub comment_type_tags_list: Vec<i32>,
}

#[derive(Message)]
pub struct Text {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub default_patter: String,
    #[prost(message, optional, tag = "3")]
    pub default_format: Option<TextFormat>,
    #[prost(message, repeated, tag = "4")]
    pub pieces_list: Vec<TextPiece>,
}

#[derive(Message)]
pub struct TextFormat {
    #[prost(string, tag = "1")]
    pub color: String,
    #[prost(bool, tag = "2")]
    pub bold: bool,
    #[prost(bool, tag = "3")]
    pub italic: bool,
    #[prost(uint32, tag = "4")]
    pub weight: u32,
    #[prost(uint32, tag = "5")]
    pub italic_angle: u32,
    #[prost(uint32, tag = "6")]
    pub font_size: u32,
    #[prost(bool, tag = "7")]
    pub use_heigh_light_color: bool,
    #[prost(bool, tag = "8")]
    pub use_remote_clor: bool,
}

#[derive(Message)]
pub struct TextPiece {
    #[prost(bool, tag = "1")]
    pub r#type: bool,
    #[prost(message, optional, tag = "2")]
    pub format: Option<TextFormat>,
    #[prost(string, tag = "3")]
    pub string_value: String,
    #[prost(message, optional, tag = "4")]
    pub user_value: Option<TextPieceUser>,
    #[prost(message, optional, tag = "5")]
    pub gift_value: Option<TextPieceGift>,
    #[prost(message, optional, tag = "6")]
    pub heart_value: Option<TextPieceHeart>,
    #[prost(message, optional, tag = "7")]
    pub pattern_ref_value: Option<TextPiecePatternRef>,
    #[prost(message, optional, tag = "8")]
    pub image_value: Option<TextPieceImage>,
}

#[derive(Message)]
pub struct TextPieceUser {
    #[prost(message, optional, tag = "1")]
    pub user: Option<User>,
    #[prost(bool, tag = "2")]
    pub with_colon: bool,
}

#[derive(Message)]
pub struct TextPieceGift {
    #[prost(uint64, tag = "1")]
    pub gift_id: u64,
    #[prost(message, optional, tag = "2")]
    pub name_ref: Option<PatternRef>,
}

#[derive(Message)]
pub struct PatternRef {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub default_pattern: String,
}

#[derive(Message)]
pub struct TextPieceHeart {
    #[prost(string, tag = "1")]
    pub color: String,
}

#[derive(Message)]
pub struct TextPiecePatternRef {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub default_pattern: String,
}

#[derive(Message)]
pub struct TextPieceImage {
    #[prost(message, optional, tag = "1")]
    pub image: Option<Image>,
    #[prost(float, tag = "2")]
    pub scaling_rate: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum CommentTypeTag {
    CommentTypeTagUnknown = 0,
    CommentTypeTagStar = 1,
}

#[derive(Message)]
pub struct DouyinChatMessage {
    #[prost(message, optional, tag = "1")]
    pub common: Option<Common>,
    #[prost(message, optional, tag = "2")]
    pub user: Option<User>,
    #[prost(string, tag = "3")]
    pub content: String,
    #[prost(bool, tag = "4")]
    pub visible_to_sender: bool,
    #[prost(message, optional, tag = "5")]
    pub background_image: Option<Image>,
    #[prost(string, tag = "6")]
    pub full_screen_text_color: String,
    #[prost(message, optional, tag = "7")]
    pub background_image_v2: Option<Image>,
    #[prost(message, optional, tag = "9")]
    pub public_area_common: Option<PublicAreaCommon>,
    #[prost(message, optional, tag = "10")]
    pub gift_image: Option<Image>,
    #[prost(uint64, tag = "11")]
    pub agree_msg_id: u64,
    #[prost(uint32, tag = "12")]
    pub priority_level: u32,
    #[prost(message, optional, tag = "13")]
    pub landscape_area_common: Option<LandscapeAreaCommon>,
    #[prost(uint64, tag = "15")]
    pub event_time: u64,
    #[prost(bool, tag = "16")]
    pub send_review: bool,
    #[prost(bool, tag = "17")]
    pub from_intercom: bool,
    #[prost(bool, tag = "18")]
    pub intercom_hide_user_card: bool,
    #[prost(string, tag = "20")]
    pub chat_by: String,
    #[prost(uint32, tag = "21")]
    pub individual_chat_priority: u32,
    #[prost(message, optional, tag = "22")]
    pub rtf_content: Option<Text>,
}

#[derive(Message)]
pub struct GiftMessage {
    #[prost(message, optional, tag = "1")]
    pub common: Option<Common>,
    #[prost(uint64, tag = "2")]
    pub gift_id: u64,
    #[prost(uint64, tag = "3")]
    pub fan_ticket_count: u64,
    #[prost(uint64, tag = "4")]
    pub group_count: u64,
    #[prost(uint64, tag = "5")]
    pub repeat_count: u64,
    #[prost(uint64, tag = "6")]
    pub combo_count: u64,
    #[prost(message, optional, tag = "7")]
    pub user: Option<User>,
    #[prost(message, optional, tag = "8")]
    pub to_user: Option<User>,
    #[prost(uint32, tag = "9")]
    pub repeat_end: u32,
    #[prost(message, optional, tag = "10")]
    pub text_effect: Option<TextEffect>,
    #[prost(uint64, tag = "11")]
    pub group_id: u64,
    #[prost(uint64, tag = "12")]
    pub income_taskgifts: u64,
    #[prost(uint64, tag = "13")]
    pub room_fan_ticket_count: u64,
    #[prost(message, optional, tag = "14")]
    pub priority: Option<GiftIMPriority>,
    #[prost(message, optional, tag = "15")]
    pub gift: Option<GiftStruct>,
    #[prost(string, tag = "16")]
    pub log_id: String,
    #[prost(uint64, tag = "17")]
    pub send_type: u64,
    #[prost(message, optional, tag = "18")]
    pub public_area_common: Option<PublicAreaCommon>,
    #[prost(message, optional, tag = "19")]
    pub tray_display_text: Option<Text>,
    #[prost(uint64, tag = "20")]
    pub banned_display_effects: u64,
    #[prost(bool, tag = "25")]
    pub display_for_self: bool,
    #[prost(string, tag = "26")]
    pub interact_gift_info: String,
    #[prost(string, tag = "27")]
    pub diy_item_info: String,
    #[prost(uint64, repeated, tag = "28")]
    pub min_asset_set_list: Vec<u64>,
    #[prost(uint64, tag = "29")]
    pub total_count: u64,
    #[prost(uint32, tag = "30")]
    pub client_gift_source: u32,
    #[prost(uint64, repeated, tag = "32")]
    pub to_user_ids_list: Vec<u64>,
    #[prost(uint64, tag = "33")]
    pub send_time: u64,
    #[prost(uint64, tag = "34")]
    pub force_display_effects: u64,
    #[prost(string, tag = "35")]
    pub trace_id: String,
    #[prost(uint64, tag = "36")]
    pub effect_display_ts: u64,
}

#[derive(Message)]
pub struct GiftStruct {
    #[prost(message, optional, tag = "1")]
    pub image: Option<Image>,
    #[prost(string, tag = "2")]
    pub describe: String,
    #[prost(bool, tag = "3")]
    pub notify: bool,
    #[prost(uint64, tag = "4")]
    pub duration: u64,
    #[prost(uint64, tag = "5")]
    pub id: u64,
    #[prost(bool, tag = "7")]
    pub for_linkmic: bool,
    #[prost(bool, tag = "8")]
    pub doodle: bool,
    #[prost(bool, tag = "9")]
    pub for_fansclub: bool,
    #[prost(bool, tag = "10")]
    pub combo: bool,
    #[prost(uint32, tag = "11")]
    pub r#type: u32,
    #[prost(uint32, tag = "12")]
    pub diamond_count: u32,
    #[prost(bool, tag = "13")]
    pub is_displayed_on_panel: bool,
    #[prost(uint64, tag = "14")]
    pub primary_effect_id: u64,
    #[prost(message, optional, tag = "15")]
    pub gift_label_icon: Option<Image>,
    #[prost(string, tag = "16")]
    pub name: String,
    #[prost(string, tag = "17")]
    pub region: String,
    #[prost(string, tag = "18")]
    pub manual: String,
    #[prost(bool, tag = "19")]
    pub for_custom: bool,
    #[prost(message, optional, tag = "21")]
    pub icon: Option<Image>,
    #[prost(uint32, tag = "22")]
    pub action_type: u32,
}

#[derive(Message)]
pub struct GiftIMPriority {
    #[prost(uint64, repeated, tag = "1")]
    pub queue_sizes_list: Vec<u64>,
    #[prost(uint64, tag = "2")]
    pub self_queue_priority: u64,
    #[prost(uint64, tag = "3")]
    pub priority: u64,
}

#[derive(Message)]
pub struct TextEffect {
    #[prost(message, optional, tag = "1")]
    pub portrait: Option<TextEffectDetail>,
    #[prost(message, optional, tag = "2")]
    pub landscape: Option<TextEffectDetail>,
}

#[derive(Message)]
pub struct TextEffectDetail {
    #[prost(message, optional, tag = "1")]
    pub text: Option<Text>,
    #[prost(uint32, tag = "2")]
    pub text_font_size: u32,
    #[prost(message, optional, tag = "3")]
    pub background: Option<Image>,
    #[prost(uint32, tag = "4")]
    pub start: u32,
    #[prost(uint32, tag = "5")]
    pub duration: u32,
    #[prost(uint32, tag = "6")]
    pub x: u32,
    #[prost(uint32, tag = "7")]
    pub y: u32,
    #[prost(uint32, tag = "8")]
    pub width: u32,
    #[prost(uint32, tag = "9")]
    pub height: u32,
    #[prost(uint32, tag = "10")]
    pub shadow_dx: u32,
    #[prost(uint32, tag = "11")]
    pub shadow_dy: u32,
    #[prost(uint32, tag = "12")]
    pub shadow_radius: u32,
    #[prost(string, tag = "13")]
    pub shadow_color: String,
    #[prost(string, tag = "14")]
    pub stroke_color: String,
    #[prost(uint32, tag = "15")]
    pub stroke_width: u32,
}

#[derive(Message)]
pub struct LikeMessage {
    #[prost(message, optional, tag = "1")]
    pub common: Option<Common>,
    #[prost(uint64, tag = "2")]
    pub count: u64,
    #[prost(uint64, tag = "3")]
    pub total: u64,
    #[prost(uint64, tag = "4")]
    pub color: u64,
    #[prost(message, optional, tag = "5")]
    pub user: Option<User>,
    #[prost(string, tag = "6")]
    pub icon: String,
    #[prost(message, optional, tag = "7")]
    pub double_like_detail: Option<DoubleLikeDetail>,
    #[prost(message, optional, tag = "8")]
    pub display_control_info: Option<DisplayControlInfo>,
    #[prost(uint64, tag = "9")]
    pub linkmic_guest_uid: u64,
    #[prost(string, tag = "10")]
    pub scene: String,
    #[prost(message, optional, tag = "11")]
    pub pico_display_info: Option<PicoDisplayInfo>,
}

#[derive(Message)]
pub struct DoubleLikeDetail {
    #[prost(bool, tag = "1")]
    pub double_flag: bool,
    #[prost(uint32, tag = "2")]
    pub seq_id: u32,
    #[prost(uint32, tag = "3")]
    pub renewals_num: u32,
    #[prost(uint32, tag = "4")]
    pub triggers_num: u32,
}

#[derive(Message)]
pub struct DisplayControlInfo {
    #[prost(bool, tag = "1")]
    pub show_text: bool,
    #[prost(bool, tag = "2")]
    pub show_icons: bool,
}

#[derive(Message)]
pub struct PicoDisplayInfo {
    #[prost(uint64, tag = "1")]
    pub combo_sum_count: u64,
    #[prost(string, tag = "2")]
    pub emoji: String,
    #[prost(message, optional, tag = "3")]
    pub emoji_icon: Option<Image>,
    #[prost(string, tag = "4")]
    pub emoji_text: String,
}

#[derive(Message)]
pub struct MemberMessage {
    #[prost(message, optional, tag = "1")]
    pub common: Option<Common>,
    #[prost(message, optional, tag = "2")]
    pub user: Option<User>,
    #[prost(uint64, tag = "3")]
    pub member_count: u64,
    #[prost(message, optional, tag = "4")]
    pub operator: Option<User>,
    #[prost(bool, tag = "5")]
    pub is_set_to_admin: bool,
    #[prost(bool, tag = "6")]
    pub is_top_user: bool,
    #[prost(uint64, tag = "7")]
    pub rank_score: u64,
    #[prost(uint64, tag = "8")]
    pub top_user_no: u64,
    #[prost(uint64, tag = "9")]
    pub enter_type: u64,
    #[prost(uint64, tag = "10")]
    pub action: u64,
    #[prost(string, tag = "11")]
    pub action_description: String,
    #[prost(uint64, tag = "12")]
    pub user_id: u64,
    #[prost(message, optional, tag = "13")]
    pub effect_config: Option<EffectConfig>,
    #[prost(string, tag = "14")]
    pub pop_str: String,
    #[prost(message, optional, tag = "15")]
    pub enter_effect_config: Option<EffectConfig>,
    #[prost(message, optional, tag = "16")]
    pub background_image: Option<Image>,
    #[prost(message, optional, tag = "17")]
    pub background_image_v2: Option<Image>,
    #[prost(message, optional, tag = "18")]
    pub anchor_display_text: Option<Text>,
    #[prost(message, optional, tag = "19")]
    pub public_area_common: Option<PublicAreaCommon>,
    #[prost(uint64, tag = "20")]
    pub user_enter_tip_type: u64,
    #[prost(uint64, tag = "21")]
    pub anchor_enter_tip_type: u64,
}

#[derive(Message)]
pub struct EffectConfig {
    #[prost(uint64, tag = "1")]
    pub r#type: u64,
    #[prost(message, optional, tag = "2")]
    pub icon: Option<Image>,
    #[prost(uint64, tag = "3")]
    pub avatar_pos: u64,
    #[prost(message, optional, tag = "4")]
    pub text: Option<Text>,
    #[prost(message, optional, tag = "5")]
    pub text_icon: Option<Image>,
    #[prost(uint32, tag = "6")]
    pub stay_time: u32,
    #[prost(uint64, tag = "7")]
    pub anim_asset_id: u64,
    #[prost(message, optional, tag = "8")]
    pub badge: Option<Image>,
    #[prost(uint64, repeated, tag = "9")]
    pub flex_setting_array_list: Vec<u64>,
    #[prost(message, optional, tag = "10")]
    pub text_icon_overlay: Option<Image>,
    #[prost(message, optional, tag = "11")]
    pub animated_badge: Option<Image>,
    #[prost(bool, tag = "12")]
    pub has_sweep_light: bool,
    #[prost(uint64, repeated, tag = "13")]
    pub text_flex_setting_array_list: Vec<u64>,
    #[prost(uint64, tag = "14")]
    pub center_anim_asset_id: u64,
    #[prost(message, optional, tag = "15")]
    pub dynamic_image: Option<Image>,
    #[prost(map = "string, string", tag = "16")]
    pub extra_map: HashMap<String, String>,
    #[prost(uint64, tag = "17")]
    pub mp4_anim_asset_id: u64,
    #[prost(uint64, tag = "18")]
    pub priority: u64,
    #[prost(uint64, tag = "19")]
    pub max_wait_time: u64,
    #[prost(string, tag = "20")]
    pub dress_id: String,
    #[prost(uint64, tag = "21")]
    pub alignment: u64,
    #[prost(uint64, tag = "22")]
    pub alignment_offset: u64,
}

// message PushFrame {
//   uint64 seqId = 1;
//   uint64 logId = 2;
//   uint64 service = 3;
//   uint64 method = 4;
//   repeated HeadersList headersList = 5;
//   string payloadEncoding = 6;
//   string payloadType = 7;
//   bytes payload = 8;
// }

#[derive(Message)]
pub struct PushFrame {
    #[prost(uint64, tag = "1")]
    pub seq_id: u64,
    #[prost(uint64, tag = "2")]
    pub log_id: u64,
    #[prost(uint64, tag = "3")]
    pub service: u64,
    #[prost(uint64, tag = "4")]
    pub method: u64,
    #[prost(message, repeated, tag = "5")]
    pub headers_list: Vec<HeadersList>,
    #[prost(string, tag = "6")]
    pub payload_encoding: String,
    #[prost(string, tag = "7")]
    pub payload_type: String,
    #[prost(bytes, tag = "8")]
    pub payload: Vec<u8>,
}

// message HeadersList {
//   string key = 1;
//   string value = 2;
// }

#[derive(Message)]
pub struct HeadersList {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub value: String,
}
