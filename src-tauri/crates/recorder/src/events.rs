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
