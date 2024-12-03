use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize, Deserialize, Debug)]
pub struct GeneralResponse {
    pub code: u8,
    pub message: String,
    pub ttl: u8,
    pub data: Data,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum Data {
    VideoSubmit(VideoSubmitData),
    Cover(CoverData),
    RoomPlayInfo(RoomPlayInfoData),
    VideoTypeList(VideoTypeListData),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct VideoSubmitData {
    pub aid: u64,
    pub bvid: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CoverData {
    pub url: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PreuploadResponse {
    pub endpoint: String,
    pub upos_uri: String,
    pub auth: String,
    pub chunk_size: usize,
    pub biz_id: u64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PostVideoMetaResponse {
    pub bucket: String,
    pub key: String,
    pub upload_id: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomPlayInfoData {
    #[serde(rename = "room_id")]
    pub room_id: i64,
    #[serde(rename = "short_id")]
    pub short_id: i64,
    pub uid: i64,
    #[serde(rename = "is_hidden")]
    pub is_hidden: bool,
    #[serde(rename = "is_locked")]
    pub is_locked: bool,
    #[serde(rename = "is_portrait")]
    pub is_portrait: bool,
    #[serde(rename = "live_status")]
    pub live_status: i64,
    #[serde(rename = "hidden_till")]
    pub hidden_till: i64,
    #[serde(rename = "lock_till")]
    pub lock_till: i64,
    pub encrypted: bool,
    #[serde(rename = "pwd_verified")]
    pub pwd_verified: bool,
    #[serde(rename = "live_time")]
    pub live_time: i64,
    #[serde(rename = "room_shield")]
    pub room_shield: i64,
    #[serde(rename = "all_special_types")]
    pub all_special_types: Vec<i64>,
    #[serde(rename = "playurl_info")]
    pub playurl_info: PlayurlInfo,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayurlInfo {
    #[serde(rename = "conf_json")]
    pub conf_json: String,
    pub playurl: Playurl,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playurl {
    pub cid: i64,
    #[serde(rename = "g_qn_desc")]
    pub g_qn_desc: Vec<GQnDesc>,
    pub stream: Vec<Stream>,
    #[serde(rename = "p2p_data")]
    pub p2p_data: P2pData,
    #[serde(rename = "dolby_qn")]
    pub dolby_qn: Value,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GQnDesc {
    pub qn: i64,
    pub desc: String,
    #[serde(rename = "hdr_desc")]
    pub hdr_desc: String,
    #[serde(rename = "attr_desc")]
    pub attr_desc: Value,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stream {
    #[serde(rename = "protocol_name")]
    pub protocol_name: String,
    pub format: Vec<Format>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Format {
    #[serde(rename = "format_name")]
    pub format_name: String,
    pub codec: Vec<Codec>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Codec {
    #[serde(rename = "codec_name")]
    pub codec_name: String,
    #[serde(rename = "current_qn")]
    pub current_qn: i64,
    #[serde(rename = "accept_qn")]
    pub accept_qn: Vec<i64>,
    #[serde(rename = "base_url")]
    pub base_url: String,
    #[serde(rename = "url_info")]
    pub url_info: Vec<UrlInfo>,
    #[serde(rename = "hdr_qn")]
    pub hdr_qn: Value,
    #[serde(rename = "dolby_type")]
    pub dolby_type: i64,
    #[serde(rename = "attr_name")]
    pub attr_name: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlInfo {
    pub host: String,
    pub extra: String,
    #[serde(rename = "stream_ttl")]
    pub stream_ttl: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct P2pData {
    pub p2p: bool,
    #[serde(rename = "p2p_type")]
    pub p2p_type: i64,
    #[serde(rename = "m_p2p")]
    pub m_p2p: bool,
    #[serde(rename = "m_servers")]
    pub m_servers: Value,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoTypeListData {
    pub typelist: Vec<Typelist>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Typelist {
    pub id: i64,
    pub parent: i64,
    #[serde(rename = "parent_name")]
    pub parent_name: String,
    pub name: String,
    pub description: String,
    pub desc: String,
    #[serde(rename = "intro_original")]
    pub intro_original: String,
    #[serde(rename = "intro_copy")]
    pub intro_copy: String,
    pub notice: String,
    #[serde(rename = "copy_right")]
    pub copy_right: i64,
    pub show: bool,
    pub rank: i64,
    pub children: Vec<Children>,
    #[serde(rename = "max_video_count")]
    pub max_video_count: i64,
    #[serde(rename = "request_id")]
    pub request_id: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Children {
    pub id: i64,
    pub parent: i64,
    #[serde(rename = "parent_name")]
    pub parent_name: String,
    pub name: String,
    pub description: String,
    pub desc: String,
    #[serde(rename = "intro_original")]
    pub intro_original: String,
    #[serde(rename = "intro_copy")]
    pub intro_copy: String,
    pub notice: String,
    #[serde(rename = "copy_right")]
    pub copy_right: i64,
    pub show: bool,
    pub rank: i64,
    #[serde(rename = "max_video_count")]
    pub max_video_count: i64,
    #[serde(rename = "request_id")]
    pub request_id: String,
}
