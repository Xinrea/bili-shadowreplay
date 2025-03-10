use async_trait::async_trait;

use super::{danmu::DanmuEntry, Recorder, RecorderType};

pub struct DouyinRecorder {
    pub room_id: u64,
    pub live_id: u64,
    pub ts_length: u64,
    pub ts_entries: Vec<super::TsEntry>,
}

#[async_trait]
impl Recorder for DouyinRecorder {
    fn recorder_type(&self) -> RecorderType {
        RecorderType::Douyin
    }

    async fn run(&self) {
        todo!()
    }

    async fn stop(&self) {
        todo!()
    }

    async fn clip_range(&self, live_id: u64, x: f64, y: f64, output_path: &str) -> Result<String, super::errors::RecorderError> {
        todo!()
    }

    async fn m3u8_content(&self, live_id: u64) -> String {
        todo!()
    }

    async fn info(&self) -> super::RecorderInfo {
        todo!()
    }

    async fn comments(&self, live_id: u64) -> Result<Vec<DanmuEntry>, super::errors::RecorderError> {
        todo!()
    }
}

