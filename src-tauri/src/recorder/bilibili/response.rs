use serde::{Deserialize, Serialize};

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
