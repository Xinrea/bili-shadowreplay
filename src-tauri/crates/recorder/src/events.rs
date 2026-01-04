use crate::platforms::PlatformType;
use crate::RecorderInfo;

#[derive(Debug, Clone)]
pub enum RecorderEvent {
    LiveStart {
        recorder: RecorderInfo,
    },
    LiveEnd {
        room_id: String,
        platform: PlatformType,
        recorder: RecorderInfo,
    },
    RecordStart {
        recorder: RecorderInfo,
    },
    RecordEnd {
        recorder: RecorderInfo,
    },
    RecordUpdate {
        live_id: String,
        duration_secs: f64,
        cached_size_bytes: u64,
    },
    ProgressUpdate {
        id: String,
        content: String,
    },
    ProgressFinished {
        id: String,
        success: bool,
        message: String,
    },
    DanmuReceived {
        room: String,
        ts: i64,
        content: String,
    },
}
