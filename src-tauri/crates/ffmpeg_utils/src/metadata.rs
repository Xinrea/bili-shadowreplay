use std::{
    ffi::{CStr, CString},
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
};

use ffmpeg_next::{self as ffmpeg, ffi, format, media, Rational};
use serde::{Deserialize, Serialize};

use crate::command::path_str;

/// The media description shared by the application and recorder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoMetadata {
    /// Positive video-track duration, falling back to the container span.
    /// Unknown and zero durations are reported as `0.0`.
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub video_codec: String,
    /// Empty when the file has no audio stream or its codec is unknown.
    pub audio_codec: String,
    /// Container bit rate in bits per second, `0` when unknown.
    pub bitrate: u64,
    /// The first video stream's `r_frame_rate`, `0.0` when unknown.
    pub fps: f64,
    /// Input size in bytes, `0` when unknown.
    pub file_size: u64,
}

impl VideoMetadata {
    /// A TS segment missing SPS/PPS still has a stream, but no dimensions.
    /// The recorder appends it behind the previous segment rather than failing.
    pub fn seems_corrupted(&self) -> bool {
        self.width == 0 && self.height == 0
    }

    /// Resolution and codecs determine stream compatibility; duration, frame
    /// rate and bit rate may legitimately vary between live segments.
    pub fn same_stream_shape(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.video_codec == other.video_codec
            && self.audio_codec == other.audio_codec
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioStreamMetadata {
    pub codec: String,
    pub sample_rate: u32,
    /// Timestamp of the first sample in seconds; absent when unknown.
    pub start_time: Option<f64>,
}

/// Includes the distinct container and stream clocks needed for preview audio
/// and overlays. They must not be replaced with the video track's duration.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaProbe {
    pub metadata: VideoMetadata,
    pub container_duration: Option<f64>,
    pub container_start_time: Option<f64>,
    pub video_start_time: Option<f64>,
    pub audio: Option<AudioStreamMetadata>,
}

/// Probe through libavformat, without starting ffprobe. Blocking demuxer work
/// stays off Tokio's executor and observes cancellation while opening/reading.
pub async fn probe_media(file_path: &Path) -> Result<MediaProbe, String> {
    run_probe(ProbeSource::File(file_path.to_path_buf())).await
}

pub async fn extract_video_metadata(file_path: &Path) -> Result<VideoMetadata, String> {
    probe_media(file_path).await.map(|probe| probe.metadata)
}

/// Probe an fMP4 media segment together with its `#EXT-X-MAP` initialization.
/// A seekable AVIO joins the two files without copying them or writing a
/// temporary sibling, so concurrent probes cannot overwrite each other's input.
pub async fn extract_video_metadata_with_init(
    init_path: &Path,
    media_path: &Path,
) -> Result<VideoMetadata, String> {
    run_probe(ProbeSource::Fragment {
        init: init_path.to_path_buf(),
        media: media_path.to_path_buf(),
    })
    .await
    .map(|probe| probe.metadata)
}

enum ProbeSource {
    File(PathBuf),
    Fragment { init: PathBuf, media: PathBuf },
}

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

async fn run_probe(source: ProbeSource) -> Result<MediaProbe, String> {
    run_media(move |cancelled| describe(&open_source(source, cancelled)?)).await
}

/// Own the cancellation signal outside the blocking task so dropping the async
/// caller interrupts the native input while the worker retains its I/O guards.
pub(crate) async fn run_media<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(Arc<AtomicBool>) -> Result<T, String> + Send + 'static,
{
    let cancel = CancelOnDrop(Arc::new(AtomicBool::new(false)));
    let token = cancel.0.clone();
    let result = tokio::task::spawn_blocking(move || work(token))
        .await
        .map_err(|error| format!("媒体处理任务失败: {error}"))?;
    drop(cancel);
    result
}

pub(crate) fn initialize() -> Result<(), String> {
    static INITIALIZED: OnceLock<Result<(), String>> = OnceLock::new();
    INITIALIZED
        .get_or_init(|| ffmpeg::init().map_err(|error| format!("初始化 FFmpeg 失败: {error}")))
        .clone()
}

/// Owns a context even before open succeeds. `avformat_open_input` may free and
/// null it on failure; close accepts null. Custom AVIO is owned separately until
/// the successfully opened context is transferred into ffmpeg-next's Input.
struct OpeningContext(*mut ffi::AVFormatContext);

impl Drop for OpeningContext {
    fn drop(&mut self) {
        // SAFETY: this pointer is either our sole context allocation or null.
        unsafe { ffi::avformat_close_input(&mut self.0) };
    }
}

pub(crate) fn open_input(
    file_path: &Path,
    cancelled: Arc<AtomicBool>,
) -> Result<format::context::Input, String> {
    open_source(ProbeSource::File(file_path.to_path_buf()), cancelled)
}

fn open_source(
    source: ProbeSource,
    cancelled: Arc<AtomicBool>,
) -> Result<format::context::Input, String> {
    initialize()?;
    if cancelled.load(Ordering::Acquire) {
        return Err("媒体探测已取消".into());
    }
    let (filename, mut io) = match source {
        ProbeSource::File(path) => {
            let filename = CString::new(path_str(&path)?)
                .map_err(|_| format!("媒体路径包含 NUL: {}", path.display()))?;
            (Some(filename), None)
        }
        ProbeSource::Fragment { init, media } => {
            let reader = JoinedFiles::open(&init, &media, cancelled.clone())?;
            let io = format::context::StreamIo::from_read_seek(reader)
                .map_err(|error| format!("创建 fMP4 AVIO 失败: {error}"))?;
            (None, Some(io))
        }
    };
    let token = cancelled.clone();
    let interrupt = ffmpeg::util::interrupt::new(Box::new(move || token.load(Ordering::Acquire)));
    // SAFETY: allocate our context and configure it before any demuxer can run.
    let mut opening = OpeningContext(unsafe { ffi::avformat_alloc_context() });
    if opening.0.is_null() {
        return Err("分配媒体探测上下文失败".into());
    }
    unsafe {
        (*opening.0).interrupt_callback = interrupt.interrupt;
        if let Some(io) = io.as_mut() {
            (*opening.0).pb = io.as_mut_ptr();
            (*opening.0).flags |= ffi::AVFMT_FLAG_CUSTOM_IO;
        }
        let result = ffi::avformat_open_input(
            &mut opening.0,
            filename.as_ref().map_or(ptr::null(), |name| name.as_ptr()),
            ptr::null(),
            ptr::null_mut(),
        );
        if result < 0 {
            return Err(format!("打开媒体失败: {}", ffmpeg::Error::from(result)));
        }
        let result = ffi::avformat_find_stream_info(opening.0, ptr::null_mut());
        if cancelled.load(Ordering::Acquire) {
            return Err("媒体探测已取消".into());
        }
        // ffprobe also keeps discovered streams when parameter discovery fails:
        // missing SPS/PPS must reach seems_corrupted(), not abort HLS recording.
        if (*opening.0).nb_streams == 0 {
            return Err(if result < 0 {
                format!("未找到视频流: {}", ffmpeg::Error::from(result))
            } else {
                "未找到视频流".into()
            });
        }
        let context = std::mem::replace(&mut opening.0, ptr::null_mut());
        let input = match io.take() {
            Some(io) => format::context::Input::wrap_with_custom_io_and_interrupt(
                context,
                io,
                interrupt.guard,
            ),
            None => format::context::Input::wrap_with_interrupt(context, interrupt.guard),
        };
        Ok(input)
    }
}

fn timestamp(value: i64, time_base: Rational) -> Option<f64> {
    if value == ffi::AV_NOPTS_VALUE || time_base.denominator() == 0 {
        return None;
    }
    let seconds =
        value as f64 * f64::from(time_base.numerator()) / f64::from(time_base.denominator());
    seconds.is_finite().then_some(seconds)
}

fn video_duration(video: Option<f64>, container: Option<f64>) -> f64 {
    video
        .filter(|duration| *duration > 0.0)
        .or_else(|| container.filter(|duration| *duration > 0.0))
        .unwrap_or(0.0)
}

fn frame_rate(rate: Rational) -> f64 {
    if rate.denominator() == 0 {
        0.0
    } else {
        f64::from(rate.numerator()) / f64::from(rate.denominator())
    }
}

fn codec_name(parameters: &ffmpeg::codec::Parameters) -> String {
    // SAFETY: parameters retain their owning format context. Descriptors have
    // static lifetimes. Match ffprobe's descriptor name, including an empty
    // codec name when no descriptor is available.
    unsafe {
        let descriptor = ffi::avcodec_descriptor_get((*parameters.as_ptr()).codec_id);
        if descriptor.is_null() {
            String::new()
        } else {
            CStr::from_ptr((*descriptor).name)
                .to_string_lossy()
                .into_owned()
        }
    }
}

fn describe(input: &format::context::Input) -> Result<MediaProbe, String> {
    if input.nb_streams() == 0 {
        return Err("未找到视频流".into());
    }
    let container_clock = Rational(1, ffi::AV_TIME_BASE);
    let container_duration = timestamp(input.duration(), container_clock);
    // SAFETY: input owns a live format context throughout this read.
    let container_start_time = unsafe { timestamp((*input.as_ptr()).start_time, container_clock) };
    let video = input
        .streams()
        .find(|stream| stream.parameters().medium() == media::Type::Video);
    let audio = input
        .streams()
        .find(|stream| stream.parameters().medium() == media::Type::Audio)
        .map(|stream| {
            let parameters = stream.parameters();
            AudioStreamMetadata {
                codec: codec_name(&parameters),
                // SAFETY: the parameters are borrowed from the live input.
                sample_rate: unsafe { (*parameters.as_ptr()).sample_rate.max(0) as u32 },
                start_time: timestamp(stream.start_time(), stream.time_base()),
            }
        });
    let video_start_time = video
        .as_ref()
        .and_then(|stream| timestamp(stream.start_time(), stream.time_base()));
    let mut metadata = VideoMetadata {
        duration: video_duration(
            video
                .as_ref()
                .and_then(|stream| timestamp(stream.duration(), stream.time_base())),
            container_duration,
        ),
        width: 0,
        height: 0,
        video_codec: String::new(),
        audio_codec: audio
            .as_ref()
            .map_or_else(String::new, |audio| audio.codec.clone()),
        bitrate: input.bit_rate().max(0) as u64,
        fps: 0.0,
        file_size: input.io_size().unwrap_or(0).max(0) as u64,
    };
    if let Some(stream) = video {
        let parameters = stream.parameters();
        // SAFETY: the codec parameters remain owned by input until this returns.
        unsafe {
            metadata.width = (*parameters.as_ptr()).width.max(0) as u32;
            metadata.height = (*parameters.as_ptr()).height.max(0) as u32;
        }
        metadata.video_codec = codec_name(&parameters);
        metadata.fps = frame_rate(stream.rate());
    }
    Ok(MediaProbe {
        metadata,
        container_duration,
        container_start_time,
        video_start_time,
        audio,
    })
}

/// The virtual input is `[init bytes][media bytes]`. Reads stay bounded to the
/// caller's AVIO buffer; Seek supports size discovery and arbitrary revisits.
struct JoinedFiles {
    files: [File; 2],
    lengths: [u64; 2],
    position: u64,
    cancelled: Arc<AtomicBool>,
}

impl JoinedFiles {
    fn open(init: &Path, media: &Path, cancelled: Arc<AtomicBool>) -> Result<Self, String> {
        let init = File::open(init).map_err(|error| format!("读取 fMP4 init 失败: {error}"))?;
        let media = File::open(media).map_err(|error| format!("读取 fMP4 media 失败: {error}"))?;
        let lengths = [
            init.metadata().map_err(|error| error.to_string())?.len(),
            media.metadata().map_err(|error| error.to_string())?.len(),
        ];
        lengths[0]
            .checked_add(lengths[1])
            .filter(|length| *length <= i64::MAX as u64)
            .ok_or("fMP4 输入过大")?;
        Ok(Self {
            files: [init, media],
            lengths,
            position: 0,
            cancelled,
        })
    }

    fn check_cancelled(&self) -> io::Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            // Interrupted is automatically retried by AVIO. Use a terminal error
            // so custom I/O also stops if libavformat doesn't poll its callback.
            Err(io::Error::other("媒体探测已取消"))
        } else {
            Ok(())
        }
    }
}

impl Read for JoinedFiles {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.check_cancelled()?;
        let total = self.lengths[0] + self.lengths[1];
        let mut written = 0;
        while written < output.len() && self.position < total {
            self.check_cancelled()?;
            let index = usize::from(self.position >= self.lengths[0]);
            let offset = self.position - if index == 1 { self.lengths[0] } else { 0 };
            let count =
                (self.lengths[index] - offset).min((output.len() - written) as u64) as usize;
            self.files[index].seek(SeekFrom::Start(offset))?;
            let read = self.files[index].read(&mut output[written..written + count])?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "fMP4 输入被截断",
                ));
            }
            self.position += read as u64;
            written += read;
        }
        Ok(written)
    }
}

impl Seek for JoinedFiles {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.check_cancelled()?;
        let position = match from {
            SeekFrom::Start(position) => i128::from(position),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
            SeekFrom::End(offset) => {
                i128::from(self.lengths[0] + self.lengths[1]) + i128::from(offset)
            }
        };
        if !(0..=i128::from(i64::MAX)).contains(&position) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "无效 fMP4 seek",
            ));
        }
        self.position = position as u64;
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/video")
            .join(name)
    }

    fn segment_fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../recorder/tests/fixtures")
            .join(name)
    }

    #[test]
    fn duration_falls_back_only_when_the_video_track_has_no_positive_length() {
        for (video, container, expected) in [
            (Some(9.5), Some(10.0), 9.5),
            (None, Some(5.023_222), 5.023_222),
            (Some(0.0), Some(4.0), 4.0),
            (Some(-1.0), Some(4.0), 4.0),
            (Some(0.0), None, 0.0),
            (None, Some(0.0), 0.0),
            (None, None, 0.0),
        ] {
            assert_eq!(video_duration(video, container), expected);
        }
    }

    #[test]
    fn timestamps_keep_zero_and_negative_starts_but_reject_nopts() {
        assert_eq!(timestamp(ffi::AV_NOPTS_VALUE, Rational(1, 1)), None);
        assert_eq!(timestamp(10, Rational(1, 0)), None);
        assert_eq!(timestamp(0, Rational(1, 48_000)), Some(0.0));
        assert_eq!(
            timestamp(-1024, Rational(1, 48_000)),
            Some(-1024.0 / 48_000.0)
        );
        assert_eq!(frame_rate(Rational(0, 0)), 0.0);
        assert_eq!(frame_rate(Rational(60_000, 1001)), 60_000.0 / 1001.0);
    }

    #[tokio::test]
    async fn probes_a_recorded_file_without_an_audio_track() {
        let metadata = extract_video_metadata(&fixture("test.mp4")).await.unwrap();
        assert!((metadata.duration - 5.45).abs() < 1e-6);
        assert_eq!((metadata.width, metadata.height), (3240, 2160));
        assert_eq!(metadata.video_codec, "h264");
        assert_eq!(metadata.audio_codec, "");
        assert_eq!(metadata.fps, 60.0);
        assert_eq!(
            metadata.file_size,
            std::fs::metadata(fixture("test.mp4")).unwrap().len()
        );
        assert!(!metadata.seems_corrupted());
    }

    #[tokio::test]
    async fn probes_audio_only_files_and_exposes_the_first_audio_clock() {
        let probe = probe_media(&fixture("../audio/test.wav")).await.unwrap();
        assert_eq!(probe.metadata.video_codec, "");
        assert_eq!((probe.metadata.width, probe.metadata.height), (0, 0));
        assert_eq!(probe.metadata.audio_codec, "pcm_s16le");
        assert_eq!(probe.metadata.duration, 83.008);
        let audio = probe.audio.unwrap();
        assert_eq!(audio.codec, "pcm_s16le");
        assert_eq!(audio.sample_rate, 16_000);
        assert_eq!(audio.start_time, None);
    }

    #[tokio::test]
    async fn probes_an_initialization_segment_without_playable_duration() {
        let metadata = extract_video_metadata(&fixture("init.m4s")).await.unwrap();
        assert_eq!(metadata.duration, 0.0);
        assert_eq!((metadata.width, metadata.height), (1920, 1080));
        assert_eq!(metadata.video_codec, "h264");
        assert_eq!(metadata.audio_codec, "aac");
    }

    #[tokio::test]
    async fn a_media_segment_without_its_initialization_is_an_error() {
        assert!(extract_video_metadata(&fixture("segment.m4s"))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn concurrent_fmp4_probes_do_not_touch_a_sibling_temp_file() {
        let directory = tempfile::tempdir().unwrap();
        let init = directory.path().join("init.m4s");
        let media = directory.path().join("segment.m4s");
        std::fs::copy(fixture("init.m4s"), &init).unwrap();
        std::fs::copy(fixture("segment.m4s"), &media).unwrap();
        let sentinel = media.with_extension("probe.tmp");
        std::fs::write(&sentinel, b"unrelated file").unwrap();
        let (a, b) = tokio::join!(
            extract_video_metadata_with_init(&init, &media),
            extract_video_metadata_with_init(&init, &media)
        );
        let a = a.unwrap();
        assert_eq!(a, b.unwrap());
        assert_eq!((a.width, a.height), (1920, 1080));
        assert_eq!(
            (a.video_codec.as_str(), a.audio_codec.as_str()),
            ("h264", "aac")
        );
        assert_eq!(a.fps, 60.0);
        assert_eq!(
            a.file_size,
            std::fs::metadata(&init).unwrap().len() + std::fs::metadata(&media).unwrap().len()
        );
        assert!(!a.seems_corrupted());
        assert_eq!(std::fs::read(sentinel).unwrap(), b"unrelated file");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 3);
    }

    #[tokio::test]
    async fn ts_segments_missing_sps_pps_reach_corruption_detection() {
        let valid = extract_video_metadata(&segment_fixture("probe-h264.ts"))
            .await
            .unwrap();
        let missing = extract_video_metadata(&segment_fixture("probe-h264-no-parameters.ts"))
            .await
            .unwrap();
        assert!(!valid.seems_corrupted());
        assert_eq!((valid.width, valid.height), (64, 48));
        assert!(missing.seems_corrupted());
        assert_eq!(missing.video_codec, "h264");
        assert!((missing.duration - 0.4).abs() < 1e-6);
        assert_eq!(missing.fps, 25.0);
    }

    #[tokio::test]
    async fn stream_shape_ignores_duration_bitrate_and_frame_rate() {
        let metadata = extract_video_metadata(&fixture("test.mp4")).await.unwrap();
        let mut other = metadata.clone();
        other.duration *= 2.0;
        other.bitrate *= 2;
        other.file_size *= 2;
        other.fps /= 2.0;
        assert!(metadata.same_stream_shape(&other));
        other.width /= 2;
        assert!(!metadata.same_stream_shape(&other));
        other = metadata.clone();
        other.video_codec = "hevc".into();
        assert!(!metadata.same_stream_shape(&other));
        other = metadata.clone();
        other.audio_codec = "aac".into();
        assert!(!metadata.same_stream_shape(&other));
    }

    #[tokio::test]
    async fn invalid_or_missing_inputs_return_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(probe_media(&dir.path().join("absent.mp4")).await.is_err());
        let invalid = dir.path().join("invalid.mp4");
        std::fs::write(&invalid, b"not a media file").unwrap();
        assert!(probe_media(&invalid).await.is_err());
    }

    #[tokio::test]
    async fn cancelling_a_stalled_http_probe_closes_its_native_connection() {
        use std::time::Duration;
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
            sync::oneshot,
            time::timeout,
        };

        // Always release the socket and abort the probe on assertion failure:
        // Tokio cannot abort a running spawn_blocking task by itself.
        struct AbortOnDrop(tokio::task::AbortHandle);
        impl Drop for AbortOnDrop {
            fn drop(&mut self) {
                self.0.abort();
            }
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (ready_tx, ready_rx) = oneshot::channel();
        let mut server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut bytes = [0; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = socket.read(&mut bytes).await.unwrap();
                assert!(read > 0 && request.len() < 16 * 1024);
                request.extend_from_slice(&bytes[..read]);
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nContent-Length: 1000000000\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            ready_tx.send(()).unwrap();
            // With no response body the native demuxer blocks in HTTP read.
            // Only cancelling/closing that client can make this return EOF.
            socket.read(&mut bytes).await
        });
        let _server_guard = AbortOnDrop(server.abort_handle());
        let url = PathBuf::from(format!("http://{address}/stalled.ts"));
        let probe = tokio::spawn(async move { probe_media(&url).await });
        let _probe_guard = AbortOnDrop(probe.abort_handle());
        timeout(Duration::from_secs(5), ready_rx)
            .await
            .expect("native probe did not open the HTTP input")
            .unwrap();
        // Let libavformat enter its body read after receiving the headers.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!probe.is_finished());
        probe.abort();
        assert!(probe.await.unwrap_err().is_cancelled());
        let closed = timeout(Duration::from_secs(5), &mut server)
            .await
            .expect("cancelled native worker retained the HTTP socket")
            .unwrap()
            .unwrap();
        assert_eq!(closed, 0);
    }

    #[test]
    fn joined_io_reads_across_parts_and_seeks_without_losing_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let init = directory.path().join("init");
        let media = directory.path().join("media");
        std::fs::write(&init, b"abcd").unwrap();
        std::fs::write(&media, b"efghi").unwrap();
        let token = Arc::new(AtomicBool::new(false));
        let mut input = JoinedFiles::open(&init, &media, token.clone()).unwrap();
        let mut bytes = [0; 6];
        input.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"abcdef");
        assert_eq!(input.seek(SeekFrom::End(-6)).unwrap(), 3);
        input.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"defghi");
        assert_eq!(input.read(&mut bytes).unwrap(), 0);
        assert!(input.seek(SeekFrom::Current(-10)).is_err());
        input.seek(SeekFrom::Start(0)).unwrap();
        token.store(true, Ordering::Release);
        assert!(input.read(&mut bytes).is_err());
        assert!(input.seek(SeekFrom::End(0)).is_err());
    }

    // The CLI is an optional oracle, never a prerequisite for native fixtures.
    #[tokio::test]
    async fn native_fields_match_ffprobe_when_available() {
        use crate::ffprobe_command;
        let available = ffprobe_command().arg("-version").output().await;
        if !available.is_ok_and(|output| output.status.success()) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let joined = dir.path().join("joined.mp4");
        let mut contents = std::fs::read(fixture("init.m4s")).unwrap();
        contents.extend(std::fs::read(fixture("segment.m4s")).unwrap());
        std::fs::write(&joined, contents).unwrap();
        for path in [
            fixture("test.mp4"),
            fixture("init.m4s"),
            fixture("../audio/test.wav"),
            joined.clone(),
            segment_fixture("probe-h264.ts"),
            segment_fixture("probe-h264-no-parameters.ts"),
        ] {
            let output = ffprobe_command()
                .args([
                    "-v",
                    "quiet",
                    "-show_streams",
                    "-show_format",
                    "-of",
                    "json",
                ])
                .arg(&path)
                .output()
                .await
                .unwrap();
            assert!(output.status.success(), "{}", path.display());
            let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            let probe = probe_media(&path).await.unwrap();
            let metadata = &probe.metadata;
            let streams = report["streams"].as_array().unwrap();
            let video = streams
                .iter()
                .find(|stream| stream["codec_type"] == "video");
            let audio = streams
                .iter()
                .find(|stream| stream["codec_type"] == "audio");
            let number = |value: &serde_json::Value| {
                value.as_str().and_then(|value| value.parse::<f64>().ok())
            };
            let close = |actual: Option<f64>, expected: Option<f64>| match (actual, expected) {
                (Some(actual), Some(expected)) => assert!(
                    (actual - expected).abs() <= 1e-6,
                    "{}: {actual} != {expected}",
                    path.display()
                ),
                (None, None) => (),
                pair => panic!("{}: {pair:?}", path.display()),
            };
            close(
                probe.container_duration,
                number(&report["format"]["duration"]),
            );
            close(
                probe.container_start_time,
                number(&report["format"]["start_time"]),
            );
            close(
                probe.video_start_time,
                video.and_then(|stream| number(&stream["start_time"])),
            );
            close(
                Some(metadata.duration),
                Some(video_duration(
                    video.and_then(|stream| number(&stream["duration"])),
                    number(&report["format"]["duration"]),
                )),
            );
            assert_eq!(
                metadata.width,
                video
                    .and_then(|stream| stream["width"].as_u64())
                    .unwrap_or(0) as u32
            );
            assert_eq!(
                metadata.height,
                video
                    .and_then(|stream| stream["height"].as_u64())
                    .unwrap_or(0) as u32
            );
            assert_eq!(
                metadata.video_codec,
                video
                    .and_then(|stream| stream["codec_name"].as_str())
                    .unwrap_or("")
            );
            assert_eq!(
                metadata.audio_codec,
                audio
                    .and_then(|stream| stream["codec_name"].as_str())
                    .unwrap_or("")
            );
            let expected_fps = video
                .and_then(|stream| stream["r_frame_rate"].as_str())
                .and_then(|rate| rate.split_once('/'))
                .and_then(|(numerator, denominator)| {
                    let numerator = numerator.parse::<f64>().ok()?;
                    let denominator = denominator.parse::<f64>().ok()?;
                    (denominator != 0.0).then_some(numerator / denominator)
                })
                .unwrap_or(0.0);
            close(Some(metadata.fps), Some(expected_fps));
            assert_eq!(
                metadata.bitrate,
                report["format"]["bit_rate"]
                    .as_str()
                    .and_then(|rate| rate.parse::<u64>().ok())
                    .unwrap_or(0)
            );
            assert_eq!(metadata.file_size, std::fs::metadata(&path).unwrap().len());
            if let (Some(probed), Some(stream)) = (&probe.audio, audio) {
                assert_eq!(
                    probed.sample_rate,
                    stream["sample_rate"]
                        .as_str()
                        .unwrap()
                        .parse::<u32>()
                        .unwrap()
                );
                close(probed.start_time, number(&stream["start_time"]));
            }
        }
        assert_eq!(
            extract_video_metadata_with_init(&fixture("init.m4s"), &fixture("segment.m4s"))
                .await
                .unwrap(),
            extract_video_metadata(&joined).await.unwrap()
        );
    }
}
