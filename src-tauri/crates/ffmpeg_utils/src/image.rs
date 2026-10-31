//! Restricted first-frame image normalization for downloaded emoji. This is a
//! deliberately narrower policy than video thumbnails: local regular files,
//! bounded encoded data/pixels/PNG metadata, one decoding/filter/encoding thread.
//! FFmpeg has no per-context allocator budget. No process-global av_max_alloc is
//! changed, and a currently executing codec call cannot be forcibly interrupted.

use std::{
    fs::File,
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    path::Path,
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use ffmpeg_next::{self as ffmpeg, codec, ffi, filter, format, frame, Packet};
use flate2::read::ZlibDecoder;

use crate::metadata::{initialize, run_media};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_PIXELS: u64 = 4_194_304;
const TIMEOUT: Duration = Duration::from_secs(10);
const FORMATS: &str = "image2,png_pipe,jpeg_pipe,webp_pipe,gif,apng";
const FILTER: &str =
    "scale=w='min(iw,512)':h='min(ih,512)':force_original_aspect_ratio=decrease,format=rgba";

/// Normalize the first frame of a PNG/APNG, JPEG, GIF or WebP file into a PNG
/// no larger than 512×512, retaining aspect ratio and alpha. Encoded inputs and
/// decompressed PNG text/ICC metadata are each limited to 8 MiB. The ten-second
/// deadline cancels publication; native codec calls finish cooperatively.
pub async fn normalize_image_to_png(source: &Path, output: &Path) -> Result<(), String> {
    normalize_with_timeout(source, output, TIMEOUT).await
}

async fn normalize_with_timeout(
    source: &Path,
    output: &Path,
    timeout: Duration,
) -> Result<(), String> {
    let source = source.to_path_buf();
    let output = output.to_path_buf();
    let target = output.clone();
    let work = run_media(move |cancelled| {
        let budget = Budget {
            cancelled,
            deadline: Instant::now() + timeout,
        };
        budget.check()?;
        let canonical_source = source.canonicalize().ok();
        if source == output
            || (canonical_source.is_some() && canonical_source == output.canonicalize().ok())
        {
            return Err("image output cannot overwrite its input".into());
        }
        let data = read_image(&source, &budget)?;
        validate_image(&data, &budget)?;
        let decoded = first_frame(data, &budget)?;
        let encoded = encode_png(decoded, &budget)?;
        let parent = output
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temporary = tempfile::Builder::new()
            .prefix(".bsr-image-")
            .suffix(".png")
            .tempfile_in(parent)
            .map_err(|error| format!("create image output: {error}"))?;
        temporary
            .write_all(&encoded)
            .map_err(|error| format!("write image output: {error}"))?;
        temporary
            .flush()
            .map_err(|error| format!("flush image output: {error}"))?;
        budget.check()?;
        Ok(temporary)
    });
    let temporary = tokio::time::timeout(timeout, work)
        .await
        .map_err(|_| "image decode timed out".to_string())??;
    // Dropping the async caller before this point drops the worker's temporary
    // file. No await separates successful work from atomic publication.
    temporary
        .persist(target)
        .map_err(|error| format!("save image output: {}", error.error))?;
    Ok(())
}

#[derive(Clone)]
struct Budget {
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
}

impl Budget {
    fn check(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err("image decode cancelled".into());
        }
        if Instant::now() >= self.deadline {
            return Err("image decode timed out".into());
        }
        Ok(())
    }

    fn check_io(&self) -> io::Result<()> {
        self.check()
            .map_err(|error| io::Error::new(io::ErrorKind::TimedOut, error))
    }
}

fn read_image(path: &Path, budget: &Budget) -> Result<Vec<u8>, String> {
    budget.check()?;
    // Inspect before opening so ordinary directories, device files and FIFOs
    // cannot turn the worker into an unbounded stream reader.
    let metadata = std::fs::metadata(path).map_err(|error| format!("read image file: {error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
        return Err("image must be a regular file of at most 8 MiB".into());
    }
    let file = File::open(path).map_err(|error| format!("open image file: {error}"))?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
        return Err("image must be a regular file of at most 8 MiB".into());
    }
    let mut data = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|error| format!("read image bytes: {error}"))?;
    budget.check()?;
    if data.len() > MAX_BYTES {
        return Err("image exceeds 8 MiB".into());
    }
    Ok(data)
}

fn dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("image exceeds the 4194304 pixel limit or has invalid dimensions".into());
    }
    Ok(())
}

fn validate_image(data: &[u8], budget: &Budget) -> Result<(), String> {
    budget.check()?;
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return validate_png(data, budget);
    }
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        let header = data.get(6..10).ok_or("truncated GIF header")?;
        return dimensions(
            u16::from_le_bytes(header[..2].try_into().unwrap()).into(),
            u16::from_le_bytes(header[2..].try_into().unwrap()).into(),
        );
    }
    if data.starts_with(b"\xff\xd8\xff") {
        return validate_jpeg(data);
    }
    if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        let length = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
        if length < 4 || length > data.len() - 8 {
            return Err("truncated WebP container".into());
        }
        if data.get(12..16) == Some(b"VP8X") {
            let header = data.get(20..30).ok_or("truncated WebP extended header")?;
            let size =
                |v: &[u8]| u32::from(v[0]) | (u32::from(v[1]) << 8) | (u32::from(v[2]) << 16);
            dimensions(size(&header[4..7]) + 1, size(&header[7..10]) + 1)?;
        }
        return Ok(());
    }
    Err("unsupported image format".into())
}

fn validate_jpeg(data: &[u8]) -> Result<(), String> {
    let mut position = 2;
    while position < data.len() {
        if data[position] != 0xff {
            return Err("invalid JPEG marker".into());
        }
        while data.get(position) == Some(&0xff) {
            position += 1;
        }
        let marker = *data.get(position).ok_or("truncated JPEG marker")?;
        position += 1;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        if marker == 0x01 || (0xd0..=0xd8).contains(&marker) {
            continue;
        }
        let size = data
            .get(position..position + 2)
            .ok_or("truncated JPEG segment")?;
        let size = u16::from_be_bytes(size.try_into().unwrap()) as usize;
        if size < 2 {
            return Err("invalid JPEG segment length".into());
        }
        let payload = data
            .get(position + 2..position + size)
            .ok_or("truncated JPEG segment")?;
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            let size = payload.get(1..5).ok_or("truncated JPEG dimensions")?;
            return dimensions(
                u16::from_be_bytes(size[2..4].try_into().unwrap()).into(),
                u16::from_be_bytes(size[..2].try_into().unwrap()).into(),
            );
        }
        position += size;
    }
    Err("JPEG has no image dimensions".into())
}

fn validate_png(data: &[u8], budget: &Budget) -> Result<(), String> {
    let mut position = 8;
    let mut header = false;
    let mut metadata_bytes = 0;
    while position < data.len() {
        budget.check()?;
        let chunk = data
            .get(position..position + 8)
            .ok_or("truncated PNG chunk")?;
        let length = u32::from_be_bytes(chunk[..4].try_into().unwrap()) as usize;
        let end = position
            .checked_add(12)
            .and_then(|v| v.checked_add(length))
            .filter(|end| *end <= data.len())
            .ok_or("truncated PNG chunk")?;
        let payload = &data[position + 8..end - 4];
        let kind = &chunk[4..8];
        if !header {
            if kind != b"IHDR" || payload.len() != 13 {
                return Err("PNG has no valid IHDR".into());
            }
            dimensions(
                u32::from_be_bytes(payload[..4].try_into().unwrap()),
                u32::from_be_bytes(payload[4..8].try_into().unwrap()),
            )?;
            header = true;
        }
        let compressed = if kind == b"iCCP" || kind == b"zTXt" {
            let split = payload
                .iter()
                .position(|byte| *byte == 0)
                .ok_or("invalid compressed PNG metadata")?;
            if payload.get(split + 1) != Some(&0) {
                return Err("invalid PNG metadata compression".into());
            }
            Some(payload.get(split + 2..).ok_or("truncated PNG metadata")?)
        } else if kind == b"iTXt" {
            let split = payload
                .iter()
                .position(|byte| *byte == 0)
                .ok_or("invalid PNG international text")?;
            let fields = payload
                .get(split + 1..)
                .ok_or("invalid PNG international text")?;
            if fields.first() == Some(&1) {
                if fields.get(1) != Some(&0) {
                    return Err("invalid PNG text compression".into());
                }
                let language = fields.get(2..).ok_or("invalid PNG text language")?;
                let split = language
                    .iter()
                    .position(|byte| *byte == 0)
                    .ok_or("invalid PNG text language")?;
                let translated = &language[split + 1..];
                let split = translated
                    .iter()
                    .position(|byte| *byte == 0)
                    .ok_or("invalid PNG translated keyword")?;
                Some(&translated[split + 1..])
            } else {
                None
            }
        } else {
            None
        };
        if let Some(compressed) = compressed {
            // PNG's decoder expands these with AVBPrint_UNLIMITED. Validate
            // their expansion before native decoding; keep the original bytes
            // and ordinary ICC metadata intact for encoder propagation.
            let mut decoder = ZlibDecoder::new(compressed);
            let mut block = [0; 8192];
            loop {
                budget.check()?;
                let size = decoder
                    .read(&mut block)
                    .map_err(|error| format!("invalid PNG compressed metadata: {error}"))?;
                if size == 0 {
                    break;
                }
                metadata_bytes += size;
                if metadata_bytes > MAX_BYTES {
                    return Err("PNG decompressed metadata exceeds 8 MiB".into());
                }
            }
        }
        if kind == b"IEND" {
            return Ok(());
        }
        position = end;
    }
    Err("PNG has no IEND".into())
}

struct ImageReader {
    data: Cursor<Vec<u8>>,
    budget: Budget,
}
impl Read for ImageReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.budget.check_io()?;
        self.data.read(bytes)
    }
}
impl Seek for ImageReader {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.budget.check_io()?;
        self.data.seek(from)
    }
}

struct Opening(*mut ffi::AVFormatContext);
impl Drop for Opening {
    fn drop(&mut self) {
        unsafe {
            ffi::avformat_close_input(&mut self.0);
        }
    }
}

fn first_frame(data: Vec<u8>, budget: &Budget) -> Result<frame::Video, String> {
    initialize()?;
    budget.check()?;
    let mut io = format::context::StreamIo::from_read_seek(ImageReader {
        data: Cursor::new(data),
        budget: budget.clone(),
    })
    .map_err(|error| format!("create image I/O: {error}"))?;
    let token = budget.clone();
    let interrupt = ffmpeg::util::interrupt::new(Box::new(move || token.check().is_err()));
    let mut opening = Opening(unsafe { ffi::avformat_alloc_context() });
    if opening.0.is_null() {
        return Err("allocate image demuxer failed".into());
    }
    let mut options = ffmpeg::Dictionary::new();
    options.set("format_whitelist", FORMATS);
    options.set("protocol_whitelist", "file");
    options.set("codec_whitelist", "png,apng,mjpeg,webp,gif");
    unsafe {
        (*opening.0).pb = io.as_mut_ptr();
        (*opening.0).flags |= ffi::AVFMT_FLAG_CUSTOM_IO;
        (*opening.0).interrupt_callback = interrupt.interrupt;
        (*opening.0).max_streams = 1;
        (*opening.0).probesize = MAX_BYTES as i64;
        let mut raw_options = options.disown();
        // No input URL is provided: even image2 cannot construct a filename
        // sequence. All encoded bytes come from the single bounded buffer.
        let result =
            ffi::avformat_open_input(&mut opening.0, ptr::null(), ptr::null(), &mut raw_options);
        let _options = ffmpeg::Dictionary::own(raw_options);
        if result < 0 {
            return Err(format!(
                "open image payload: {}",
                ffmpeg::Error::from(result)
            ));
        }
    }
    budget.check()?;
    let context = std::mem::replace(&mut opening.0, ptr::null_mut());
    let mut input = unsafe {
        format::context::Input::wrap_with_custom_io_and_interrupt(context, io, interrupt.guard)
    };
    // Avoid find_stream_info's implicit decoder, which would run before the
    // per-decoder pixel/thread limits are installed. These demuxers identify
    // their image codec in read_header; dimensions are learned while decoding.
    let stream = input
        .streams()
        .find(|stream| stream.parameters().medium() == ffmpeg::media::Type::Video)
        .ok_or("image payload has no video stream")?;
    let index = stream.index();
    if !matches!(
        stream.parameters().id(),
        codec::Id::PNG | codec::Id::APNG | codec::Id::MJPEG | codec::Id::GIF | codec::Id::WEBP
    ) {
        return Err("unsupported image codec".into());
    }
    let mut context = codec::context::Context::from_parameters(stream.parameters())
        .map_err(|error| format!("configure image decoder: {error}"))?;
    unsafe {
        (*context.as_mut_ptr()).max_pixels = MAX_PIXELS as i64;
        (*context.as_mut_ptr()).thread_count = 1;
    }
    let mut decoder = context.decoder();
    decoder.set_packet_time_base(stream.time_base());
    let mut decoder = decoder
        .video()
        .map_err(|error| format!("open image decoder: {error}"))?;
    let mut decoded = frame::Video::empty();
    loop {
        budget.check()?;
        let mut packet = Packet::empty();
        match packet.read(&mut input) {
            Ok(()) if packet.stream() != index => continue,
            Ok(()) => {}
            Err(ffmpeg::Error::Eof) => break,
            Err(error) => return Err(format!("read image packet: {error}")),
        }
        if packet.size() > MAX_BYTES {
            return Err("image packet exceeds 8 MiB".into());
        }
        decoder
            .send_packet(&packet)
            .map_err(|error| format!("decode image packet: {error}"))?;
        if receive_image(&mut decoder, &mut decoded, budget)? {
            return Ok(decoded);
        }
    }
    decoder
        .send_eof()
        .map_err(|error| format!("finish image decode: {error}"))?;
    if receive_image(&mut decoder, &mut decoded, budget)? {
        return Ok(decoded);
    }
    Err("image decoder produced no frame".into())
}

fn receive_image(
    decoder: &mut codec::decoder::Video,
    decoded: &mut frame::Video,
    budget: &Budget,
) -> Result<bool, String> {
    budget.check()?;
    match decoder.receive_frame(decoded) {
        Ok(()) => {
            budget.check()?;
            dimensions(decoded.width(), decoded.height())?;
            Ok(true)
        }
        Err(ffmpeg::Error::Eof) => Ok(false),
        Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => Ok(false),
        Err(error) => Err(format!("decode image frame: {error}")),
    }
}

fn encode_png(mut decoded: frame::Video, budget: &Budget) -> Result<Vec<u8>, String> {
    budget.check()?;
    dimensions(decoded.width(), decoded.height())?;
    let mut graph = filter::Graph::new();
    unsafe {
        (*graph.as_mut_ptr()).nb_threads = 1;
    }
    let source = filter::find("buffer").ok_or("FFmpeg lacks buffer filter")?;
    let sink = filter::find("buffersink").ok_or("FFmpeg lacks buffersink filter")?;
    let sar = decoded.aspect_ratio();
    let (sar_num, sar_den) = if sar.numerator() > 0 && sar.denominator() > 0 {
        (sar.numerator(), sar.denominator())
    } else {
        (0, 1)
    };
    let mut args = format!(
        "video_size={}x{}:pix_fmt={}:time_base=1/25:pixel_aspect={sar_num}/{sar_den}",
        decoded.width(),
        decoded.height(),
        unsafe { (*decoded.as_ptr()).format }
    );
    if crate::video_frame::filter_has_option(&source, c"colorspace") {
        args.push_str(&format!(":colorspace={}", unsafe {
            (*decoded.as_ptr()).colorspace as i32
        }));
    }
    if crate::video_frame::filter_has_option(&source, c"range") {
        args.push_str(&format!(":range={}", unsafe {
            (*decoded.as_ptr()).color_range as i32
        }));
    }
    // PNG's RGBA decoder declares straight alpha on newer FFmpeg versions.
    // Other image decoders may leave their frame's mode unspecified; preserve
    // that rather than declaring a mismatching filter-link mode.
    if crate::video_frame::filter_has_option(&source, c"alpha_mode")
        && decoded.format() == ffmpeg::format::Pixel::RGBA
    {
        args.push_str(":alpha_mode=straight");
    }
    graph
        .add(&source, "in", &args)
        .map_err(|error| format!("create image filter source: {error}"))?;
    graph
        .add(&sink, "out", "")
        .map_err(|error| format!("create image filter sink: {error}"))?;
    graph
        .output("in", 0)
        .and_then(|parser| parser.input("out", 0))
        .and_then(|parser| parser.parse(FILTER))
        .map_err(|error| format!("create image normalization graph: {error}"))?;
    graph
        .validate()
        .map_err(|error| format!("configure image normalization graph: {error}"))?;
    decoded.set_pts(Some(0));
    graph
        .get("in")
        .ok_or("image filter source disappeared")?
        .source()
        .add(&decoded)
        .map_err(|error| format!("filter image frame: {error}"))?;
    let mut filtered = frame::Video::empty();
    graph
        .get("out")
        .ok_or("image filter sink disappeared")?
        .sink()
        .frame(&mut filtered)
        .map_err(|error| format!("read normalized frame: {error}"))?;
    budget.check()?;
    if filtered.width() > 512
        || filtered.height() > 512
        || filtered.format() != ffmpeg::format::Pixel::RGBA
    {
        return Err("normalized image has invalid dimensions or pixel format".into());
    }
    let codec = ffmpeg::encoder::find(codec::Id::PNG).ok_or("FFmpeg lacks PNG encoder")?;
    let mut encoder = codec::context::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|error| format!("create PNG encoder: {error}"))?;
    encoder.set_width(filtered.width());
    encoder.set_height(filtered.height());
    encoder.set_format(filtered.format());
    encoder.set_aspect_ratio(filtered.aspect_ratio());
    encoder.set_time_base((1, 25));
    encoder.set_colorspace(filtered.color_space());
    encoder.set_color_range(filtered.color_range());
    encoder.set_color_primaries(filtered.color_primaries());
    encoder.set_color_transfer_characteristic(filtered.color_transfer_characteristic());
    unsafe {
        (*encoder.as_mut_ptr()).thread_count = 1;
    }
    let mut encoder = encoder
        .open_as(codec)
        .map_err(|error| format!("open PNG encoder: {error}"))?;
    encoder
        .send_frame(&filtered)
        .map_err(|error| format!("encode PNG: {error}"))?;
    encoder
        .send_eof()
        .map_err(|error| format!("finish PNG encoding: {error}"))?;
    let mut packet = Packet::empty();
    encoder
        .receive_packet(&mut packet)
        .map_err(|error| format!("read PNG output: {error}"))?;
    budget.check()?;
    packet
        .data()
        .filter(|data| !data.is_empty())
        .map(Vec::from)
        .ok_or("PNG encoder produced no frame".into())
}

#[cfg(test)]
mod tests;
