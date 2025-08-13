pub struct StreamSegment {
    pub url: String,
    pub timestamp_ms: u64,
    pub is_header: bool,
}

pub enum StreamMessage {
    Start(StreamStart),
    HeaderChange(StreamHeaderChange),
    Update(StreamUpdate),
    Error(StreamError),
    End(StreamEnd),
}

pub enum StreamType {
    TS,
    FMP4,
}

pub struct StreamStart {
    pub room_id: String,
    pub live_id: String,
    pub title: String,
    pub cover: String,
    pub start_time_sec: i64,
    pub stream_type: StreamType,
    pub fmp4_header: Option<StreamSegment>,
}

pub struct StreamHeaderChange {
    pub segment: StreamSegment,
}

pub struct StreamUpdate {
    pub segment: StreamSegment,
}

pub struct StreamEnd {
    pub room_id: String,
    pub live_id: String,
    pub title: String,
}

pub struct StreamError {
    pub error: String,
}
