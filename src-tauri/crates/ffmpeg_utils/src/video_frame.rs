//! One-frame JPEG extraction with FFmpeg's automatic video selection, accurate
//! input seek, autorotation and MJPEG format negotiation.

use std::{
    ffi::CString,
    io::Write,
    path::Path,
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use ffmpeg_next::{self as ffmpeg, codec, ffi, filter, format, frame, Packet};

use crate::metadata::{open_input, run_media};

/// Extract the first frame at or after an input-relative timestamp. `None`
/// retains MJPEG's default rate control; `Some(2)` is the former `-q:v 2`.
pub async fn extract_jpeg(
    input: &Path,
    timestamp: f64,
    quality: Option<u32>,
) -> Result<Vec<u8>, String> {
    let timestamp = parse_timestamp(timestamp)?;
    validate_quality(quality)?;
    let input = input.to_path_buf();
    run_media(move |cancelled| extract_sync(&input, timestamp, quality, cancelled)).await
}

/// Save a JPEG only after extraction and all writes have succeeded. The worker
/// owns the temporary file, so cancellation cannot publish an incomplete image.
pub async fn write_jpeg(
    input: &Path,
    timestamp: f64,
    quality: Option<u32>,
    output: &Path,
) -> Result<(), String> {
    let timestamp = parse_timestamp(timestamp)?;
    validate_quality(quality)?;
    let input = input.to_path_buf();
    let output = output.to_path_buf();
    let target = output.clone();
    let temporary = run_media(move |cancelled| {
        let canonical_input = input.canonicalize().ok();
        if input == output
            || (canonical_input.is_some() && canonical_input == output.canonicalize().ok())
        {
            return Err("JPEG 输出不能覆盖输入媒体".into());
        }
        let data = extract_sync(&input, timestamp, quality, cancelled.clone())?;
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temporary = tempfile::Builder::new()
            .prefix(".bsr-frame-")
            .suffix(".jpg")
            .tempfile_in(parent)
            .map_err(|error| format!("创建 JPEG 临时文件失败: {error}"))?;
        temporary
            .write_all(&data)
            .map_err(|error| format!("写入 JPEG 失败: {error}"))?;
        temporary
            .flush()
            .map_err(|error| format!("写入 JPEG 失败: {error}"))?;
        check_cancelled(&cancelled)?;
        Ok(temporary)
    })
    .await?;
    // No await between receiving the worker-owned file and publishing it.
    temporary
        .persist(target)
        .map_err(|error| format!("保存 JPEG 失败: {}", error.error))?;
    Ok(())
}

fn parse_timestamp(value: f64) -> Result<i64, String> {
    if !value.is_finite() || value < 0.0 {
        return Err("视频提帧时间必须是非负有限数".into());
    }
    let argument = CString::new(value.to_string()).map_err(|error| error.to_string())?;
    let mut micros = 0;
    if unsafe { ffi::av_parse_time(&mut micros, argument.as_ptr(), 1) } < 0 || micros < 0 {
        return Err("视频提帧时间无效".into());
    }
    Ok(micros)
}

fn validate_quality(quality: Option<u32>) -> Result<(), String> {
    if quality.is_some_and(|value| !(1..=31).contains(&value)) {
        return Err("JPEG 质量必须在 1 到 31 之间".into());
    }
    Ok(())
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Acquire) {
        Err("视频提帧已取消".into())
    } else {
        Ok(())
    }
}

fn native_error(context: &str, error: ffmpeg::Error) -> String {
    format!("{context}: {error}")
}

fn again(error: ffmpeg::Error) -> bool {
    matches!(error, ffmpeg::Error::Other { errno } if errno == ffmpeg::error::EAGAIN)
}

/// Match fftools' auto-video mapping for an ordinary image output, including
/// default tracks, newly seen packets, and attached cover art.
fn video_stream(input: &format::context::Input) -> Option<usize> {
    let mut best = None;
    let mut best_score = 0;
    for stream in input.streams() {
        if stream.parameters().medium() != ffmpeg::media::Type::Video
            || stream.discard() == ffmpeg::Discard::All
        {
            continue;
        }
        let raw = unsafe { &*stream.as_ptr() };
        let parameters = unsafe { &*stream.parameters().as_ptr() };
        // AV_CODEC_PROP_ENHANCEMENT was introduced after FFmpeg 6.1. Keep
        // references to its newer enum definitions out of the common ABI.
        let descriptor = unsafe { ffi::avcodec_descriptor_get(parameters.codec_id) };
        if codec::version() >> 16 >= 63
            && !descriptor.is_null()
            && unsafe { (*descriptor).props & (1 << 5) != 0 }
        {
            continue;
        }
        let mut score = i64::from(parameters.width) * i64::from(parameters.height);
        if raw.event_flags & ffi::AVSTREAM_EVENT_FLAG_NEW_PACKETS != 0 {
            score += 100_000_000;
        }
        if stream
            .disposition()
            .contains(format::stream::Disposition::DEFAULT)
        {
            score += 5_000_000;
        }
        if stream
            .disposition()
            .contains(format::stream::Disposition::ATTACHED_PIC)
        {
            score = 1;
        }
        if score > best_score {
            best_score = score;
            best = Some(stream.index());
        }
    }
    best
}

const MICROS: ffi::AVRational = ffi::AVRational {
    num: 1,
    den: ffi::AV_TIME_BASE,
};

fn extract_sync(
    path: &Path,
    start: i64,
    quality: Option<u32>,
    cancelled: Arc<AtomicBool>,
) -> Result<Vec<u8>, String> {
    let mut input = open_input(path, cancelled.clone())?;
    let index = video_stream(&input).ok_or("媒体没有可提取的视频流")?;
    let origin = unsafe { (*input.as_ptr()).start_time };
    let target = start
        .checked_add(if origin == ffi::AV_NOPTS_VALUE {
            0
        } else {
            origin
        })
        .ok_or("视频定位时间溢出")?;
    let needs_preroll = unsafe { (*input.format().as_ptr()).flags & ffi::AVFMT_SEEK_TO_PTS == 0 }
        && input
            .streams()
            .any(|stream| unsafe { (*stream.parameters().as_ptr()).video_delay != 0 });
    let seek = if needs_preroll {
        target
            .checked_sub(3 * i64::from(ffi::AV_TIME_BASE) / 23)
            .ok_or("视频定位时间溢出")?
    } else {
        target
    };
    // CLI seeking can fail on image and non-seekable demuxers. Accurate frame
    // trimming still runs; failures while actually reading are reported below.
    // FFmpeg 6's image2 seek(0) drops its only buffered image, unlike FFmpeg
    // 9. The structured API consistently supports extracting a still image at
    // zero; image2 already starts at its first image without a seek.
    if start != 0 || input.format().name() != "image2" {
        let _seek_result = input.seek(seek, ..seek);
    }
    check_cancelled(&cancelled)?;
    let stream = input.stream(index).ok_or("找不到视频流")?;
    let base: ffi::AVRational = stream.time_base().into();
    if base.num <= 0 || base.den <= 0 {
        return Err("视频流时基无效".into());
    }
    let rotation = stream
        .side_data()
        .find(|side| side.kind() == ffmpeg::packet::side_data::Type::DisplayMatrix)
        .and_then(|side| display_matrix(side.data()));
    let rate = unsafe {
        ffi::av_guess_frame_rate(
            input.as_ptr() as *mut _,
            stream.as_ptr() as *mut _,
            ptr::null_mut(),
        )
    };
    let context = codec::context::Context::from_parameters(stream.parameters())
        .map_err(|error| native_error("初始化视频解码器失败", error))?;
    let mut decoder = context.decoder();
    decoder.set_packet_time_base(stream.time_base());
    let mut decoder = decoder
        .video()
        .map_err(|error| native_error("打开视频解码器失败", error))?;
    let offset = unsafe {
        ffi::av_rescale_q(
            target.checked_neg().ok_or("视频定位时间溢出")?,
            MICROS,
            base,
        )
    };
    let mut clock = VideoClock {
        last: None,
        duration: 1,
        base,
        rate,
    };
    let mut decoded = frame::Video::empty();
    loop {
        check_cancelled(&cancelled)?;
        let mut packet = Packet::empty();
        match packet.read(&mut input) {
            Ok(()) if packet.stream() != index => continue,
            Ok(()) => {}
            Err(ffmpeg::Error::Eof) => break,
            Err(error) => return Err(native_error("读取视频数据失败", error)),
        }
        for (timestamp, pts) in [(packet.pts(), true), (packet.dts(), false)] {
            if let Some(timestamp) = timestamp {
                let timestamp = timestamp.checked_add(offset).ok_or("视频时间戳溢出")?;
                if pts {
                    packet.set_pts(Some(timestamp));
                } else {
                    packet.set_dts(Some(timestamp));
                }
            }
        }
        loop {
            match decoder.send_packet(&packet) {
                Ok(()) => break,
                Err(error) if again(error) => {
                    if receive_frame(&mut decoder, &mut decoded, &mut clock, &cancelled)? {
                        return encode_frame(decoded, base, rate, rotation, quality, &cancelled);
                    }
                }
                Err(error) => return Err(native_error("发送视频数据失败", error)),
            }
        }
        if receive_frame(&mut decoder, &mut decoded, &mut clock, &cancelled)? {
            return encode_frame(decoded, base, rate, rotation, quality, &cancelled);
        }
    }
    loop {
        match decoder.send_eof() {
            Ok(()) | Err(ffmpeg::Error::Eof) => break,
            Err(error) if again(error) => {
                if receive_frame(&mut decoder, &mut decoded, &mut clock, &cancelled)? {
                    return encode_frame(decoded, base, rate, rotation, quality, &cancelled);
                }
            }
            Err(error) => return Err(native_error("结束视频解码失败", error)),
        }
    }
    if receive_frame(&mut decoder, &mut decoded, &mut clock, &cancelled)? {
        return encode_frame(decoded, base, rate, rotation, quality, &cancelled);
    }
    Err("指定时间没有可提取的视频帧".into())
}

struct VideoClock {
    last: Option<i64>,
    duration: i64,
    base: ffi::AVRational,
    rate: ffi::AVRational,
}

fn receive_frame(
    decoder: &mut codec::decoder::Video,
    decoded: &mut frame::Video,
    clock: &mut VideoClock,
    cancelled: &AtomicBool,
) -> Result<bool, String> {
    loop {
        check_cancelled(cancelled)?;
        match decoder.receive_frame(decoded) {
            Ok(()) => {
                let pts = decoded.timestamp().unwrap_or_else(|| {
                    clock
                        .last
                        .unwrap_or(-clock.duration)
                        .saturating_add(clock.duration)
                });
                let duration = unsafe { (*decoded.as_ptr()).duration };
                clock.duration = if duration > 0 {
                    duration
                } else if let Some(previous) = clock.last.filter(|previous| pts > *previous) {
                    pts.saturating_sub(previous)
                } else if clock.rate.num > 0 && clock.rate.den > 0 {
                    unsafe { ffi::av_rescale_q(1, ffi::av_inv_q(clock.rate), clock.base) }.max(1)
                } else {
                    clock.duration
                };
                clock.last = Some(pts);
                decoded.set_pts(Some(pts));
                unsafe {
                    (*decoded.as_mut_ptr()).time_base = clock.base;
                }
                if pts >= 0 {
                    return Ok(true);
                }
            }
            Err(ffmpeg::Error::Eof) => return Ok(false),
            Err(error) if again(error) => return Ok(false),
            Err(error) => return Err(native_error("解码视频帧失败", error)),
        }
    }
}

fn display_matrix(data: &[u8]) -> Option<[i32; 9]> {
    let bytes = data.get(..36)?;
    Some(std::array::from_fn(|i| {
        i32::from_ne_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
    }))
}

fn rotation_filters(matrix: Option<[i32; 9]>) -> Vec<String> {
    let Some(matrix) = matrix else {
        return Vec::new();
    };
    let theta = unsafe { -ffi::av_display_rotation_get(matrix.as_ptr()) }.round();
    if !theta.is_finite() {
        return Vec::new();
    }
    let theta = theta - 360.0 * (theta / 360.0 + 0.9 / 360.0).floor();
    if (theta - 90.0).abs() < 1.0 {
        vec![format!(
            "transpose={}",
            if matrix[3] > 0 {
                "cclock_flip"
            } else {
                "clock"
            }
        )]
    } else if (theta - 180.0).abs() < 1.0 {
        let mut filters = Vec::new();
        if matrix[0] < 0 {
            filters.push("hflip".into());
        }
        if matrix[4] < 0 {
            filters.push("vflip".into());
        }
        filters
    } else if (theta - 270.0).abs() < 1.0 {
        vec![format!(
            "transpose={}",
            if matrix[3] < 0 {
                "clock_flip"
            } else {
                "cclock"
            }
        )]
    } else if theta.abs() > 1.0 {
        vec![format!("rotate={theta:.6}*PI/180")]
    } else if matrix[4] < 0 {
        vec!["vflip".into()]
    } else {
        Vec::new()
    }
}

pub(crate) fn filter_has_option(filter: &filter::Filter, option: &std::ffi::CStr) -> bool {
    let mut class = unsafe { (*filter.as_ptr()).priv_class };
    !class.is_null()
        && unsafe {
            !ffi::av_opt_find(
                (&mut class as *mut *const ffi::AVClass).cast(),
                option.as_ptr(),
                ptr::null(),
                0,
                ffi::AV_OPT_SEARCH_FAKE_OBJ,
            )
            .is_null()
        }
}

fn encode_frame(
    mut decoded: frame::Video,
    base: ffi::AVRational,
    rate: ffi::AVRational,
    stream_rotation: Option<[i32; 9]>,
    quality: Option<u32>,
    cancelled: &AtomicBool,
) -> Result<Vec<u8>, String> {
    check_cancelled(cancelled)?;
    let codec = ffmpeg::encoder::find(codec::Id::MJPEG).ok_or("FFmpeg 缺少 MJPEG 编码器")?;
    let source = filter::find("buffer").ok_or("FFmpeg 缺少 buffer 滤镜")?;
    let sink = filter::find("buffersink").ok_or("FFmpeg 缺少 buffersink 滤镜")?;
    let format = filter::find("format").ok_or("FFmpeg 缺少 format 滤镜")?;
    let full_range_option = filter_has_option(&format, c"color_ranges");
    let sar = decoded.aspect_ratio();
    let sar = if sar.numerator() > 0 && sar.denominator() > 0 {
        sar
    } else {
        ffmpeg::Rational(0, 1)
    };
    let mut args = format!(
        "video_size={}x{}:pix_fmt={}:time_base={}/{}:pixel_aspect={}/{}",
        decoded.width(),
        decoded.height(),
        unsafe { (*decoded.as_ptr()).format },
        base.num,
        base.den,
        sar.numerator(),
        sar.denominator()
    );
    if filter_has_option(&source, c"colorspace") {
        args.push_str(&format!(":colorspace={}", unsafe {
            (*decoded.as_ptr()).colorspace as i32
        }));
    }
    if filter_has_option(&source, c"range") {
        args.push_str(&format!(":range={}", unsafe {
            (*decoded.as_ptr()).color_range as i32
        }));
    }
    let rotation = decoded
        .side_data(frame::side_data::Type::DisplayMatrix)
        .and_then(|side| display_matrix(side.data()))
        .or(stream_rotation);
    let mut filters = rotation_filters(rotation);
    let pixel_formats = codec
        .video()
        .map_err(|error| native_error("读取 MJPEG 格式失败", error))?
        .formats()
        .ok_or("MJPEG 编码器未声明像素格式")?
        .filter(|format| {
            full_range_option
                || matches!(
                    format,
                    ffmpeg::format::Pixel::YUVJ420P
                        | ffmpeg::format::Pixel::YUVJ422P
                        | ffmpeg::format::Pixel::YUVJ444P
                )
        })
        .map(|format| {
            format
                .descriptor()
                .map(|descriptor| descriptor.name().to_owned())
                .ok_or("无效 MJPEG 像素格式")
        })
        .collect::<Result<Vec<_>, _>>()?;
    if pixel_formats.is_empty() {
        return Err("MJPEG 编码器没有可用像素格式".into());
    }
    filters.push(format!(
        "format=pix_fmts={}{}",
        pixel_formats.join("|"),
        if full_range_option {
            ":color_ranges=pc"
        } else {
            ""
        }
    ));
    let mut graph = filter::Graph::new();
    unsafe {
        (*graph.as_mut_ptr()).nb_threads = 1;
    }
    graph
        .add(&source, "in", &args)
        .map_err(|error| native_error("创建视频滤镜输入失败", error))?;
    graph
        .add(&sink, "out", "")
        .map_err(|error| native_error("创建视频滤镜输出失败", error))?;
    graph
        .output("in", 0)
        .and_then(|parser| parser.input("out", 0))
        .and_then(|parser| parser.parse(&filters.join(",")))
        .map_err(|error| native_error("创建 JPEG 滤镜失败", error))?;
    graph
        .validate()
        .map_err(|error| native_error("配置 JPEG 滤镜失败", error))?;
    // The pixels have been autorotated; do not propagate an orientation to the
    // encoded output that could make another consumer rotate them a second time.
    decoded.remove_side_data(frame::side_data::Type::DisplayMatrix);
    graph
        .get("in")
        .ok_or("视频滤镜输入丢失")?
        .source()
        .add(&decoded)
        .map_err(|error| native_error("发送视频滤镜帧失败", error))?;
    let mut filtered = frame::Video::empty();
    graph
        .get("out")
        .ok_or("视频滤镜输出丢失")?
        .sink()
        .frame(&mut filtered)
        .map_err(|error| native_error("读取 JPEG 滤镜帧失败", error))?;
    check_cancelled(cancelled)?;
    let mut encoder = codec::context::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|error| native_error("创建 JPEG 编码器失败", error))?;
    encoder.set_width(filtered.width());
    encoder.set_height(filtered.height());
    encoder.set_format(filtered.format());
    encoder.set_aspect_ratio(filtered.aspect_ratio());
    encoder.set_colorspace(filtered.color_space());
    encoder.set_color_range(filtered.color_range());
    encoder.set_color_primaries(filtered.color_primaries());
    encoder.set_color_transfer_characteristic(filtered.color_transfer_characteristic());
    encoder.set_time_base(if rate.num > 0 && rate.den > 0 {
        ffmpeg::Rational(rate.den, rate.num)
    } else {
        ffmpeg::Rational(1, 25)
    });
    unsafe {
        (*encoder.as_mut_ptr()).thread_count = 0;
        if let Some(quality) = quality {
            (*encoder.as_mut_ptr()).flags |= ffi::AV_CODEC_FLAG_QSCALE as i32;
            (*encoder.as_mut_ptr()).global_quality = quality as i32 * ffi::FF_QP2LAMBDA;
        }
        (*filtered.as_mut_ptr()).quality = (*encoder.as_ptr()).global_quality;
    }
    let mut encoder = encoder
        .open_as(codec)
        .map_err(|error| native_error("打开 JPEG 编码器失败", error))?;
    filtered.set_pts(Some(0));
    encoder
        .send_frame(&filtered)
        .map_err(|error| native_error("编码 JPEG 失败", error))?;
    encoder
        .send_eof()
        .map_err(|error| native_error("结束 JPEG 编码失败", error))?;
    let mut packet = Packet::empty();
    encoder
        .receive_packet(&mut packet)
        .map_err(|error| native_error("读取 JPEG 数据失败", error))?;
    let data = packet
        .data()
        .filter(|data| !data.is_empty())
        .ok_or("JPEG 编码器没有输出图像")?
        .to_vec();
    check_cancelled(cancelled)?;
    Ok(data)
}

#[cfg(test)]
mod tests;
