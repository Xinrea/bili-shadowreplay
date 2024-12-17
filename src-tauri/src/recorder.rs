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
    fn recorder_type(&self) -> RecorderType;
    fn check_status(&self) -> Result<bool, errors::RecorderError>;
    fn update_entries(&self) -> Result<(), errors::RecorderError>;
}
