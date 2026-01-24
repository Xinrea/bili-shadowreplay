use serde::{Deserialize, Serialize};

/// Response structure for Kuaishou live stream data
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveStreamResponse {
    pub live_stream: Option<LiveStream>,
    pub author: Option<Author>,
    pub config: Option<Config>,
    #[serde(rename = "errorType")]
    pub error_type: Option<ErrorType>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveStream {
    #[serde(rename = "playUrls")]
    pub play_urls: Option<PlayUrls>,
    #[serde(default, alias = "coverUrl", alias = "poster", alias = "cover")]
    pub cover_url: Option<String>,
    #[serde(default, alias = "caption", alias = "title")]
    pub caption: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub caption: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayUrls {
    pub h264: Option<H264>,
    pub h265: Option<H264>, // Reusing H264 struct as structure is likely identical
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct H264 {
    #[serde(rename = "adaptationSet")]
    pub adaptation_set: Option<AdaptationSet>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdaptationSet {
    pub representation: Vec<Representation>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Representation {
    pub url: String,
    pub name: Option<String>,
    #[serde(default)]
    pub quality_type: Option<String>,
    #[serde(default)]
    pub bitrate: Option<i64>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    #[serde(default, alias = "user_name", alias = "userName")]
    pub name: String,
    #[serde(default, alias = "user_id", alias = "userId")]
    pub id: String,
    #[serde(default, alias = "headurl", alias = "headUrl", alias = "avatar")]
    pub head_url: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorType {
    pub title: String,
    pub content: String,
}

/// API response for mobile endpoint
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MobileApiResponse {
    #[serde(default)]
    pub result: Option<i64>,
    #[serde(default)]
    pub error_msg: Option<String>,
    #[serde(rename = "liveStream")]
    pub live_stream: Option<MobileLiveStream>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MobileLiveStream {
    pub living: bool,
    #[serde(rename = "hlsPlayUrl")]
    pub hls_play_url: Option<String>,
    #[serde(rename = "playUrls")]
    pub play_urls: Option<Vec<MobilePlayUrl>>,
    #[serde(rename = "multiResolutionPlayUrls")]
    pub multi_resolution_play_urls: Option<Vec<MultiResolutionPlayUrl>>,
    #[serde(rename = "multiResolutionHlsPlayUrls")]
    pub multi_resolution_hls_play_urls: Option<Vec<MultiResolutionHls>>,
    pub user: Option<MobileUser>,
    #[serde(default)]
    pub caption: Option<String>,
    #[serde(default, alias = "coverUrl", alias = "poster")]
    pub cover_url: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MobilePlayUrl {
    pub url: String,
    pub quality: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiResolutionPlayUrl {
    pub urls: Vec<String>,
    pub level: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiResolutionHls {
    pub urls: Vec<String>,
    pub level: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MobileUser {
    #[serde(rename = "user_name")]
    pub user_name: String,
    #[serde(rename = "user_id")]
    pub user_id: String,
}
