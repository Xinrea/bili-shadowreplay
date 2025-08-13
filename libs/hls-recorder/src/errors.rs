use custom_error::custom_error;

custom_error! {pub RecorderError
    IndexNotFound {url: String} = "Index not found: {url}",
    ArchiveInUse {live_id: String} = "Can not delete current stream: {live_id}",
    EmptyCache = "Cache is empty",
    M3u8ParseFailed {content: String } = "Parse m3u8 content failed: {content}",
    NoStreamAvailable = "No available stream provided",
    FreezedStream {stream: BiliStream} = "Stream is freezed: {stream}",
    StreamExpired {stream: BiliStream} = "Stream is nearly expired: {stream}",
    NoRoomInfo = "No room info provided",
    InvalidStream {stream: BiliStream} = "Invalid stream: {stream}",
    SlowStream {stream: BiliStream} = "Stream is too slow: {stream}",
    EmptyHeader = "Header url is empty",
    HeaderChanged = "Header URL changed, need to restart recording",
    InvalidTimestamp = "Header timestamp is invalid",
    ClientError {err: String} = "Client error: {err}",
    IoError {err: std::io::Error} = "IO error: {err}",
    DanmuStreamError {err: danmu_stream::DanmuStreamError} = "Danmu stream error: {err}",
    SubtitleNotFound {live_id: String} = "Subtitle not found: {live_id}",
    SubtitleGenerationFailed {error: String} = "Subtitle generation failed: {error}",
}
