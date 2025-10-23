use crate::platforms::PlatformType;
use crate::RecorderInfo;

#[derive(Debug, Clone)]
pub enum RecorderEvent {
    LiveStart {
        recorder: RecorderInfo,
    },
    LiveEnd {
        room_id: i64,
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
        duration_secs: u64,
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
        room: i64,
        ts: i64,
        content: String,
    },
}
