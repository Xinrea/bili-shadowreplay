pub mod bilibili;
pub mod errors;
pub mod stream;

pub enum RecorderType {
    BiliBili,
    Huya,
    Douyu,
    Douyin,
    Youtube,
    Twitch,
}

pub trait Recorder {
    fn get_type(&self) -> RecorderType;
}
