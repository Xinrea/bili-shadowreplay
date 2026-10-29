use super::{
    audio::AudioStage,
    subtitle::SubtitleStage,
    util::{self, Limits},
    video::VideoStage,
    *,
};
use crate::{
    audio::{audio_stream, copy_chapters, input_metadata, remux::AudioPacketClock},
    metadata::{open_input, run_media},
    output::NativeOutput,
    video_frame::video_stream,
};
use ffmpeg_next::{self as ffmpeg, codec, ffi, format, media, Packet, Rational};
use std::{
    collections::VecDeque,
    ffi::{CStr, CString},
    path::Path,
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;

pub(super) async fn run(
    plan: MediaPlan,
    progress: Option<MediaProgressCallback>,
) -> Result<MediaJobResult, String> {
    let mut completed = run_media(move |cancelled| {
        validate(&plan)?;
        execute(plan, progress, cancelled)
    })
    .await?;
    completed
        .temporary
        .persist(&completed.target)
        .map_err(|e| format!("发布媒体文件失败: {}", e.error))?;
    completed.progress.emit(MediaStage::Completed, true);
    Ok(completed.result)
}
pub(super) async fn probe(options: VideoEncodeOptions) -> Result<(), String> {
    run_media(move |cancelled| {
        validate_video(&options)?;
        crate::metadata::initialize()?;
        VideoStage::probe(options, cancelled)
    })
    .await
}

fn validate_video(options: &VideoEncodeOptions) -> Result<(), String> {
    let codec = ffmpeg::encoder::find_by_name(&options.encoder)
        .ok_or_else(|| format!("找不到视频编码器 {}", options.encoder))?;
    if codec.medium() != media::Type::Video {
        return Err("视频路由必须使用视频编码器".into());
    }
    util::validate_options(codec, &options.options)?;
    if options.tag.as_ref().is_some_and(|tag| tag.len() != 4) {
        return Err("视频编码标签必须为4字节".into());
    }
    if options.buffer_size.is_some_and(|v| v > i32::MAX as u64)
        || options.gop.is_some_and(|v| v > i32::MAX as u32)
        || options.threads.is_some_and(|v| v > i32::MAX as usize)
        || options.bit_rate.is_some_and(|v| v > i64::MAX as u64)
        || options.max_rate.is_some_and(|v| v > i64::MAX as u64)
    {
        return Err("视频编码参数超出范围".into());
    }
    if let FrameRatePolicy::Constant { num, den } = options.frame_rate {
        if num == 0 || den == 0 || num > i32::MAX as u32 || den > i32::MAX as u32 {
            return Err("视频帧率无效".into());
        }
    }
    if let Some(device) = &options.hardware_device {
        if device.kind.is_empty()
            || device.kind.as_bytes().contains(&0)
            || device
                .device
                .as_ref()
                .is_some_and(|d| d.as_bytes().contains(&0))
        {
            return Err("硬件设备配置无效".into());
        }
    }
    Ok(())
}
fn validate(plan: &MediaPlan) -> Result<(), String> {
    if plan.inputs.len() != 1 || plan.outputs.len() != 1 {
        return Err("当前执行器仅支持一个输入和一个输出".into());
    }
    if plan.routes.is_empty() {
        return Err("媒体计划必须至少指定一个流路由".into());
    }
    let input = &plan.inputs[0];
    let output = &plan.outputs[0];
    if input.path == output.path
        || input
            .path
            .canonicalize()
            .ok()
            .zip(output.path.canonicalize().ok())
            .is_some_and(|(a, b)| a == b)
    {
        return Err("媒体输出不能覆盖输入".into());
    }
    if let Some(seek) = input.seek {
        util::time(seek.seconds)?;
    }
    for value in [input.duration, output.seek, output.duration]
        .into_iter()
        .flatten()
    {
        util::time(value)?;
    }
    util::options(&output.options)?;
    crate::metadata::initialize()?;
    for route in &plan.routes {
        if route.input != 0 || route.output != 0 {
            return Err("媒体路由引用了不存在的输入或输出".into());
        }
        match &route.mode {
            StreamMode::Copy => (),
            StreamMode::Video(options) => validate_video(options)?,
            StreamMode::Audio(options) => {
                let codec = ffmpeg::encoder::find_by_name(&options.encoder)
                    .ok_or_else(|| format!("找不到音频编码器 {}", options.encoder))?;
                if codec.medium() != media::Type::Audio {
                    return Err("音频路由必须使用音频编码器".into());
                }
                util::validate_options(codec, &options.options)?;
                if options.bit_rate.is_some_and(|v| v > i64::MAX as u64)
                    || options
                        .sample_rate
                        .is_some_and(|v| v == 0 || v > i32::MAX as u32)
                {
                    return Err("音频编码参数超出范围".into());
                }
            }
            StreamMode::Subtitle(options) => {
                if let Some(name) = &options.encoder {
                    let codec = ffmpeg::encoder::find_by_name(name)
                        .ok_or_else(|| format!("找不到字幕编码器 {name}"))?;
                    if codec.medium() != media::Type::Subtitle {
                        return Err("字幕路由必须使用字幕编码器".into());
                    }
                }
            }
        }
    }
    Ok(())
}

struct Completed {
    temporary: NamedTempFile,
    target: std::path::PathBuf,
    result: MediaJobResult,
    progress: Progress,
}
struct Progress {
    callback: Option<MediaProgressCallback>,
    start: Instant,
    last: Instant,
    processed: f64,
    total: Option<f64>,
}
impl Progress {
    fn new(callback: Option<MediaProgressCallback>, total: Option<f64>) -> Self {
        let now = Instant::now();
        Self {
            callback,
            start: now,
            last: now - Duration::from_secs(1),
            processed: 0.0,
            total,
        }
    }
    fn packet(&mut self, packet: &Packet, base: Rational) {
        if let Some(pts) = packet.pts().or(packet.dts()) {
            let seconds = (pts as f64 + packet.duration().max(0) as f64)
                * f64::from(base.numerator())
                / f64::from(base.denominator());
            if seconds.is_finite() {
                self.processed = self.processed.max(seconds.max(0.0));
            }
        }
        self.emit(MediaStage::Processing, false);
    }
    fn emit(&mut self, stage: MediaStage, force: bool) {
        if !force && self.last.elapsed() < Duration::from_millis(100) {
            return;
        }
        self.last = Instant::now();
        if let Some(callback) = self.callback.as_mut() {
            let elapsed = self.start.elapsed().as_secs_f64();
            callback(MediaProgress {
                processed: self.processed,
                total: self.total,
                speed: (elapsed > 0.0).then_some(self.processed / elapsed),
                stage,
            });
        }
    }
}

struct CopyRoute {
    parameters: codec::Parameters,
    clock: AudioPacketClock,
    packet_clock: super::packet_clock::PacketClock,
    medium: media::Type,
    started: bool,
    done: bool,
}
enum RouteMode {
    Copy(CopyRoute),
    Video(Box<VideoStage>),
    Audio(Box<AudioStage>),
    Subtitle(Box<SubtitleStage>),
}
struct Route {
    input: usize,
    output: usize,
    base: Rational,
    mode: RouteMode,
    metadata: ffmpeg::Dictionary<'static>,
}
impl Route {
    fn ready(&self) -> bool {
        match &self.mode {
            RouteMode::Copy(_) => true,
            RouteMode::Video(s) => s.ready(),
            RouteMode::Audio(s) => s.ready(),
            RouteMode::Subtitle(_) => true,
        }
    }
    fn done(&self) -> bool {
        match &self.mode {
            RouteMode::Copy(s) => s.done,
            RouteMode::Video(s) => s.done(),
            RouteMode::Audio(s) => s.done(),
            RouteMode::Subtitle(s) => s.done(),
        }
    }
    fn parameters(&self) -> Option<codec::Parameters> {
        match &self.mode {
            RouteMode::Copy(s) => Some(s.parameters.clone()),
            RouteMode::Video(s) => s.parameters(),
            RouteMode::Audio(s) => s.parameters(),
            RouteMode::Subtitle(s) => Some(s.parameters()),
        }
    }
    fn encoder_base(&self) -> Rational {
        match &self.mode {
            RouteMode::Copy(_) => self.base,
            RouteMode::Video(s) => s.encoder_base(),
            RouteMode::Audio(s) => s.encoder_base(),
            RouteMode::Subtitle(s) => s.time_base(),
        }
    }
}

struct Mux {
    native: NativeOutput,
    header: bool,
    pending: VecDeque<(usize, Packet, Option<Rational>)>,
    pending_bytes: usize,
    options: ffmpeg::Dictionary<'static>,
    last_dts: Vec<Option<i64>>,
    flags: i32,
}
impl Mux {
    fn header(
        &mut self,
        routes: &mut [Route],
        input: &format::context::Input,
    ) -> Result<(), String> {
        if self.header || routes.iter().any(|r| !r.ready()) {
            return Ok(());
        }
        let tags = unsafe { (*self.native.output.format().as_ptr()).codec_tag };
        for route in routes.iter_mut() {
            let parameters = route.parameters().ok_or("媒体路由没有输出编码参数")?;
            let mut target = self
                .native
                .output
                .stream_mut(route.output)
                .ok_or("输出媒体流不存在")?;
            target.set_parameters(parameters);
            target.set_time_base(route.encoder_base());
            let source = input.stream(route.input).ok_or("输入媒体流不存在")?;
            let skip_matrix = matches!(&route.mode, RouteMode::Video(stage) if stage.autorotate());
            util::side_data(&source, &mut target, skip_matrix)?;
            if matches!(route.mode, RouteMode::Copy(_)) {
                // fftools keeps the input's nominal and average rate during
                // stream copy. Recomputing them from a trimmed packet span
                // changes Matroska DefaultDuration and the playback timeline.
                target.set_rate(source.rate());
                target.set_avg_frame_rate(source.avg_frame_rate());
                unsafe {
                    let raw = &mut *target.as_mut_ptr();
                    let parameters = &mut *raw.codecpar;
                    let mut supported_tag = 0;
                    // Keep a compatible source tag (including custom tags
                    // where the muxer has no known replacement), as fftools
                    // does when the caller did not override codec_tag.
                    if !tags.is_null()
                        && ffi::av_codec_get_id(tags, parameters.codec_tag) != parameters.codec_id
                        && ffi::av_codec_get_tag2(tags, parameters.codec_id, &mut supported_tag)
                            != 0
                    {
                        parameters.codec_tag = 0;
                    }
                    if parameters.codec_type == ffi::AVMediaType::AVMEDIA_TYPE_VIDEO {
                        let aspect = (*source.as_ptr()).sample_aspect_ratio;
                        if aspect.num != 0 {
                            parameters.sample_aspect_ratio = aspect;
                        }
                        raw.sample_aspect_ratio = parameters.sample_aspect_ratio;
                    }
                    if parameters.codec_id == ffi::AVCodecID::AV_CODEC_ID_AC3
                        || (parameters.codec_id == ffi::AVCodecID::AV_CODEC_ID_MP3
                            && matches!(parameters.block_align, 1 | 576 | 1152))
                    {
                        parameters.block_align = 0;
                    }
                }
            }
            if let RouteMode::Video(stage) = &route.mode {
                target.set_avg_frame_rate(stage.frame_rate());
            }
            target.set_metadata(std::mem::take(&mut route.metadata));
            unsafe {
                (*target.as_mut_ptr()).disposition = (*source.as_ptr()).disposition;
            }
        }
        let options = std::mem::take(&mut self.options);
        let remaining = self
            .native
            .output
            .write_header_with(options)
            .map_err(|e| util::error("写入媒体文件头失败", e))?;
        util::unused(&remaining, "封装器")?;
        self.header = true;
        Ok(())
    }
    fn queue(
        &mut self,
        route: usize,
        packet: Packet,
        encoded_base: Option<Rational>,
    ) -> Result<(), String> {
        self.pending_bytes = self
            .pending_bytes
            .checked_add(packet.size())
            .ok_or("媒体启动队列溢出")?;
        self.pending.push_back((route, packet, encoded_base));
        if self.pending_bytes > 64 * 1024 * 1024 || (!self.header && self.pending.len() > 1024) {
            return Err("媒体流参数尚未齐备，启动复用队列超出上限".into());
        }
        Ok(())
    }

    fn emit_encoded(
        &mut self,
        route: usize,
        output: usize,
        mut packet: Packet,
        base: Rational,
        progress: &mut Progress,
        cancelled: &AtomicBool,
    ) -> Result<(), String> {
        util::check(cancelled)?;
        if !self.header {
            return self.queue(route, packet, Some(base));
        }
        let output_base = self
            .native
            .output
            .stream(output)
            .ok_or("输出媒体流不存在")?
            .time_base();
        packet.rescale_ts(base, output_base);
        self.write(output, packet, progress, cancelled)
    }

    fn submit(
        &mut self,
        route: usize,
        packets: Vec<Packet>,
        routes: &mut [Route],
        input: &format::context::Input,
        progress: &mut Progress,
        cancelled: &AtomicBool,
    ) -> Result<(), String> {
        self.header(routes, input)?;
        for packet in packets {
            self.queue(route, packet, None)?;
        }
        if !self.header {
            return Ok(());
        }
        while let Some((index, mut packet, encoded_base)) = self.pending.pop_front() {
            util::check(cancelled)?;
            self.pending_bytes -= packet.size();
            let route = &mut routes[index];
            let output_base = self
                .native
                .output
                .stream(route.output)
                .ok_or("输出媒体流不存在")?
                .time_base();
            if let Some(base) = encoded_base {
                packet.rescale_ts(base, output_base);
            } else {
                match &mut route.mode {
                    RouteMode::Copy(state) if state.medium == media::Type::Audio => state
                        .clock
                        .rescale(&mut packet, route.base, output_base, &state.parameters)?,
                    _ => packet.rescale_ts(route.encoder_base(), output_base),
                }
            }
            self.write(route.output, packet, progress, cancelled)?;
        }
        Ok(())
    }

    fn write(
        &mut self,
        output: usize,
        mut packet: Packet,
        progress: &mut Progress,
        cancelled: &AtomicBool,
    ) -> Result<(), String> {
        util::check(cancelled)?;
        let output_base = self
            .native
            .output
            .stream(output)
            .ok_or("输出媒体流不存在")?
            .time_base();
        packet.set_stream(output);
        packet.set_position(-1);
        if self.flags & ffi::AVFMT_NOTIMESTAMPS == 0 {
            if let (Some(dts), Some(last)) = (packet.dts(), self.last_dts[output]) {
                let minimum = last
                    .checked_add(i64::from(self.flags & ffi::AVFMT_TS_NONSTRICT == 0))
                    .ok_or("媒体DTS溢出")?;
                if dts < minimum {
                    if packet.pts().is_some_and(|pts| pts >= dts) {
                        packet.set_pts(Some(packet.pts().unwrap().max(minimum)));
                    }
                    packet.set_dts(Some(minimum));
                }
            }
        }
        self.last_dts[output] = packet.dts();
        progress.packet(&packet, output_base);
        packet
            .write_interleaved(&mut self.native.output)
            .map_err(|e| util::error(&format!("写入输出0流{output}失败"), e))
    }
}

fn format_name(output: &MediaOutput) -> Result<String, String> {
    let filename = CString::new(crate::path_str(&output.path)?).map_err(|e| e.to_string())?;
    let name = output
        .format
        .as_ref()
        .map(|s| CString::new(s.as_bytes()))
        .transpose()
        .map_err(|e| e.to_string())?;
    let format = unsafe {
        ffi::av_guess_format(
            name.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            filename.as_ptr(),
            std::ptr::null(),
        )
    };
    if format.is_null() {
        return Err("无法确定输出媒体封装格式".into());
    }
    if unsafe { (*format).flags & ffi::AVFMT_NOFILE != 0 } {
        return Err("当前媒体计划只支持单文件封装器".into());
    }
    unsafe { CStr::from_ptr((*format).name) }
        .to_str()
        .map(str::to_owned)
        .map_err(|e| e.to_string())
}
fn selected(
    input: &format::context::Input,
    selector: StreamSelector,
    subtitle_codec: codec::Id,
    force_subtitle: bool,
) -> Option<usize> {
    match selector {
        StreamSelector::AutoVideo => video_stream(input),
        StreamSelector::AutoAudio => audio_stream(input),
        StreamSelector::FirstVideo => input
            .streams()
            .find(|s| s.parameters().medium() == media::Type::Video)
            .map(|s| s.index()),
        StreamSelector::FirstAudio => input
            .streams()
            .find(|s| s.parameters().medium() == media::Type::Audio)
            .map(|s| s.index()),
        StreamSelector::AutoSubtitle => {
            // A global `-c copy` does not force subtitle selection. fftools
            // requires a default subtitle encoder for this muxer, then picks
            // the first stream with compatible text/bitmap properties.
            if force_subtitle {
                return input
                    .streams()
                    .find(|s| s.parameters().medium() == media::Type::Subtitle)
                    .map(|s| s.index());
            }
            let encoder = ffmpeg::encoder::find(subtitle_codec)?;
            let output = unsafe { ffi::avcodec_descriptor_get(encoder.id().into()) };
            input.streams().find_map(|stream| {
                let parameters = stream.parameters();
                if parameters.medium() != media::Type::Subtitle {
                    return None;
                }
                unsafe {
                    let source = ffi::avcodec_descriptor_get(parameters.id().into());
                    let mask = ffi::AV_CODEC_PROP_TEXT_SUB | ffi::AV_CODEC_PROP_BITMAP_SUB;
                    let input_props = if source.is_null() {
                        0
                    } else {
                        (*source).props & mask
                    };
                    let output_props = if output.is_null() {
                        0
                    } else {
                        (*output).props & mask
                    };
                    (input_props & output_props != 0
                        || (!source.is_null()
                            && !output.is_null()
                            && ((*source).props == 0 || (*output).props == 0)))
                        .then_some(stream.index())
                }
            })
        }
        StreamSelector::Index(index) => input.stream(index).map(|s| s.index()),
    }
}

fn execute(
    plan: MediaPlan,
    callback: Option<MediaProgressCallback>,
    cancelled: Arc<AtomicBool>,
) -> Result<Completed, String> {
    let source = &plan.inputs[0];
    let target = &plan.outputs[0];
    let mut input = open_input(&source.path, cancelled.clone())?;
    let origin = unsafe { (*input.as_ptr()).start_time };
    let origin = if origin == ffi::AV_NOPTS_VALUE {
        0
    } else {
        origin
    };
    let start = source
        .seek
        .map(|s| util::time(s.seconds))
        .transpose()?
        .unwrap_or(0);
    let seek_target = origin.checked_add(start).ok_or("媒体定位时间溢出")?;
    if source.seek.is_some() {
        let preroll = unsafe { (*input.format().as_ptr()).flags & ffi::AVFMT_SEEK_TO_PTS == 0 }
            && input
                .streams()
                .any(|s| unsafe { (*s.parameters().as_ptr()).video_delay > 0 });
        let seek = if preroll {
            seek_target
                .checked_sub(3 * i64::from(ffi::AV_TIME_BASE) / 23)
                .ok_or("媒体定位时间溢出")?
        } else {
            seek_target
        };
        let _seek_result = input.seek(seek, ..seek);
    }
    util::check(&cancelled)?;
    let offset_us = if source.copy_timestamps {
        0
    } else {
        seek_target.checked_neg().ok_or("媒体时间戳溢出")?
    };
    let output_start = target.seek.map(util::time).transpose()?.unwrap_or(0);
    let limits = Limits {
        input_start: source.seek.filter(|s| s.accurate).map(|_| {
            if source.copy_timestamps {
                seek_target
            } else {
                0
            }
        }),
        input_duration: source.duration.map(util::time).transpose()?,
        output_start,
        output_seek: target.seek.is_some(),
        output_duration: target.duration.map(util::time).transpose()?,
    };
    let mut total = (input.duration() > 0).then(|| {
        (input.duration() as f64 / f64::from(ffi::AV_TIME_BASE) - start as f64 / 1_000_000.0)
            .max(0.0)
    });
    if let Some(duration) = source.duration {
        total = Some(total.map_or(duration, |v| v.min(duration)));
    }
    total = total.map(|v| (v - output_start as f64 / 1_000_000.0).max(0.0));
    if let Some(duration) = target.duration {
        total = Some(total.map_or(duration, |v| v.min(duration)));
    }
    let mut progress = Progress::new(callback, total);
    progress.emit(MediaStage::Processing, true);
    let name = format_name(target)?;
    let parent = target
        .path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let suffix = target
        .path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| format!(".{s}"))
        .unwrap_or_else(|| ".tmp".into());
    let temporary = tempfile::Builder::new()
        .prefix(".media-job-")
        .suffix(&suffix)
        .tempfile_in(parent)
        .map_err(|e| util::error("创建媒体临时输出失败", e))?;
    let mut native = NativeOutput::new(temporary.path(), &name, cancelled.clone())?;
    let flags = unsafe { (*native.output.format().as_ptr()).flags };
    let global_header = native
        .output
        .format()
        .flags()
        .contains(format::flag::Flags::GLOBAL_HEADER);
    unsafe {
        (*native.output.as_mut_ptr()).avoid_negative_ts = match target.negative_ts {
            NegativeTs::Auto => ffi::AVFMT_AVOID_NEG_TS_AUTO,
            NegativeTs::Disabled => ffi::AVFMT_AVOID_NEG_TS_DISABLED,
            NegativeTs::MakeZero => ffi::AVFMT_AVOID_NEG_TS_MAKE_ZERO,
        };
    }
    let mut routes = Vec::new();
    for route in &plan.routes {
        let default_subtitle_codec = unsafe { (*native.output.format().as_ptr()).subtitle_codec };
        let subtitle_encoder = match &route.mode {
            StreamMode::Subtitle(options) => match &options.encoder {
                Some(name) => ffmpeg::encoder::find_by_name(name),
                None => ffmpeg::encoder::find(default_subtitle_codec.into()),
            },
            _ => None,
        };
        let force_subtitle =
            matches!(&route.mode, StreamMode::Subtitle(options) if options.encoder.is_some());
        let subtitle_codec =
            subtitle_encoder.map_or(default_subtitle_codec.into(), |codec| codec.id());
        let Some(index) = selected(&input, route.selector, subtitle_codec, force_subtitle) else {
            if route.required {
                return Err(format!("输入0没有匹配流 {:?}", route.selector));
            } else {
                continue;
            }
        };
        if routes.iter().any(|route: &Route| route.input == index) {
            return Err("当前单输入执行器不支持重复路由同一输入流".into());
        }
        let stream = input.stream(index).ok_or("输入媒体流不存在")?;
        let base = stream.time_base();
        if base.numerator() <= 0 || base.denominator() <= 0 {
            return Err("输入媒体流时基无效".into());
        }
        let parameters = stream.parameters();
        let (_, mut metadata) = input_metadata(&input, index)?;
        let mode = match &route.mode {
            StreamMode::Copy => RouteMode::Copy(CopyRoute {
                medium: parameters.medium(),
                parameters: parameters.clone(),
                clock: AudioPacketClock::new(),
                packet_clock: super::packet_clock::PacketClock::new(&stream),
                started: false,
                done: false,
            }),
            StreamMode::Audio(options) => {
                if parameters.medium() != media::Type::Audio {
                    return Err("音频路由选择了非音频流".into());
                }
                RouteMode::Audio(Box::new(AudioStage::new(
                    parameters,
                    base,
                    options.clone(),
                    limits,
                    global_header,
                    cancelled.clone(),
                )?))
            }
            StreamMode::Video(options) => {
                if parameters.medium() != media::Type::Video {
                    return Err("视频路由选择了非视频流".into());
                }
                let rotation = stream
                    .side_data()
                    .find(|s| s.kind() == ffmpeg::packet::side_data::Type::DisplayMatrix)
                    .and_then(|s| crate::video_frame::display_matrix(s.data()));
                let rate = unsafe {
                    ffi::av_guess_frame_rate(
                        input.as_ptr().cast_mut(),
                        stream.as_ptr().cast_mut(),
                        std::ptr::null_mut(),
                    )
                };
                RouteMode::Video(Box::new(VideoStage::new(
                    parameters,
                    base,
                    rate.into(),
                    stream.avg_frame_rate(),
                    rotation,
                    options.clone(),
                    limits,
                    global_header,
                    flags,
                    input.nb_streams() == 1 || source.copy_timestamps,
                    name == "avi",
                    unsafe { (*input.format().as_ptr()).flags & ffi::AVFMT_NOTIMESTAMPS != 0 },
                    cancelled.clone(),
                )?))
            }
            StreamMode::Subtitle(_) => {
                if parameters.medium() != media::Type::Subtitle {
                    return Err("字幕路由选择了非字幕流".into());
                }
                let encoder = subtitle_encoder.ok_or("输出封装器没有可用的默认字幕编码器")?;
                metadata.set("encoder", &encoder_tag(encoder.name()));
                RouteMode::Subtitle(Box::new(SubtitleStage::new(
                    parameters,
                    base,
                    encoder,
                    limits,
                    global_header,
                    cancelled.clone(),
                )?))
            }
        };
        if let StreamMode::Audio(options) = &route.mode {
            metadata.set("encoder", &encoder_tag(&options.encoder));
        }
        if let StreamMode::Video(options) = &route.mode {
            metadata.set("encoder", &encoder_tag(&options.encoder));
        }
        let mut output = native
            .output
            .add_stream(ffmpeg::encoder::find(codec::Id::None))
            .map_err(|e| util::error("创建输出媒体流失败", e))?;
        let output_index = output.index();
        if let RouteMode::Copy(copy) = &mode {
            output.set_parameters(copy.parameters.clone());
            unsafe {
                (*(*output.as_mut_ptr()).codecpar).codec_tag = 0;
            }
        }
        output.set_time_base(base);
        routes.push(Route {
            input: index,
            output: output_index,
            base,
            mode,
            metadata,
        });
    }
    if routes.is_empty() {
        return Err("媒体计划没有选中任何可输出的流".into());
    }
    let (global, _) = input_metadata(&input, routes[0].input)?;
    native.output.set_metadata(global);
    copy_chapters(
        &input,
        &mut native.output,
        output_start
            .checked_sub(offset_us)
            .ok_or("媒体章节时间戳溢出")?,
    )?;
    let mut mux_options = target.options.clone();
    if target.faststart {
        mux_options
            .entry("movflags".into())
            .and_modify(|s| s.push_str("+faststart"))
            .or_insert_with(|| "+faststart".into());
    }
    let mut mux = Mux {
        native,
        header: false,
        pending: VecDeque::new(),
        pending_bytes: 0,
        options: util::options(&mux_options)?,
        last_dts: vec![None; routes.len()],
        flags,
    };
    mux.header(&mut routes, &input)?;
    loop {
        util::check(&cancelled)?;
        if routes.iter().all(Route::done) {
            break;
        }
        let mut packet = Packet::empty();
        match packet.read(&mut input) {
            Ok(()) => (),
            Err(ffmpeg::Error::Eof) => break,
            Err(e) => return Err(util::error("读取输入0媒体包失败", e)),
        }
        let Some(index) = routes.iter().position(|r| r.input == packet.stream()) else {
            continue;
        };
        if routes[index].done() {
            continue;
        }
        let offset =
            unsafe { ffi::av_rescale_q(offset_us, util::MICROS, routes[index].base.into()) };
        if let Some(pts) = packet.pts() {
            packet.set_pts(Some(pts.checked_add(offset).ok_or("媒体PTS溢出")?));
        }
        if let Some(dts) = packet.dts() {
            packet.set_dts(Some(dts.checked_add(offset).ok_or("媒体DTS溢出")?));
        }
        let route_base = routes[index].base;
        let output_index = routes[index].output;
        let mut emit = |packet, base| {
            mux.emit_encoded(index, output_index, packet, base, &mut progress, &cancelled)
        };
        let packets = match &mut routes[index].mode {
            RouteMode::Video(stage) => {
                stage.packet(&packet, &mut emit)?;
                Vec::new()
            }
            RouteMode::Audio(stage) => {
                stage.packet(&packet, &mut emit)?;
                Vec::new()
            }
            RouteMode::Subtitle(stage) => {
                stage.packet(&packet, &mut emit)?;
                Vec::new()
            }
            RouteMode::Copy(state) => {
                let source_stream = input.stream(packet.stream()).ok_or("输入媒体流不存在")?;
                let parser_repeat = unsafe {
                    let parser = ffi::av_stream_get_parser(source_stream.as_ptr().cast_mut());
                    (!parser.is_null()).then(|| (*parser).repeat_pict)
                };
                let timestamp = state.packet_clock.update(
                    &packet,
                    route_base,
                    &state.parameters,
                    parser_repeat,
                )?;
                let mut end = limits
                    .output_duration
                    .map(|duration| output_start.checked_add(duration).ok_or("媒体复制范围溢出"))
                    .transpose()?;
                if let Some(duration) = limits.input_duration {
                    let input_end = (if source.copy_timestamps {
                        seek_target
                    } else {
                        0
                    })
                    .checked_add(duration)
                    .ok_or("输入媒体复制范围溢出")?;
                    end = Some(end.map_or(input_end, |end| end.min(input_end)));
                }
                if end.is_some_and(|end| timestamp >= end) {
                    state.done = true;
                    Vec::new()
                } else if (!state.started && target.seek.is_some() && timestamp < output_start)
                    || (!state.started && !packet.is_key())
                {
                    Vec::new()
                } else {
                    state.started = true;
                    if packet.dts().is_none() {
                        packet.set_dts(Some(unsafe {
                            ffi::av_rescale_q(timestamp, util::MICROS, route_base.into())
                        }));
                    }
                    let offset =
                        unsafe { ffi::av_rescale_q(output_start, util::MICROS, route_base.into()) };
                    if let Some(pts) = packet.pts() {
                        packet.set_pts(Some(pts.checked_sub(offset).ok_or("媒体PTS溢出")?));
                    }
                    if let Some(dts) = packet.dts() {
                        packet.set_dts(Some(dts.checked_sub(offset).ok_or("媒体DTS溢出")?));
                    }
                    vec![packet]
                }
            }
        };
        mux.submit(
            index,
            packets,
            &mut routes,
            &input,
            &mut progress,
            &cancelled,
        )?;
    }
    for index in 0..routes.len() {
        let output_index = routes[index].output;
        let mut emit = |packet, base| {
            mux.emit_encoded(index, output_index, packet, base, &mut progress, &cancelled)
        };
        let packets = match &mut routes[index].mode {
            RouteMode::Copy(state) => {
                state.done = true;
                Vec::new()
            }
            RouteMode::Video(stage) => {
                stage.finish(&mut emit)?;
                Vec::new()
            }
            RouteMode::Audio(stage) => {
                stage.finish(&mut emit)?;
                Vec::new()
            }
            RouteMode::Subtitle(stage) => {
                stage.finish(&mut emit)?;
                Vec::new()
            }
        };
        mux.submit(
            index,
            packets,
            &mut routes,
            &input,
            &mut progress,
            &cancelled,
        )?;
    }
    if !mux.header {
        return Err("媒体流没有提供可写出的编码参数".into());
    }
    progress.emit(MediaStage::Finalizing, true);
    util::check(&cancelled)?;
    mux.native
        .output
        .write_trailer()
        .map_err(|e| util::error("写入媒体文件尾失败", e))?;
    mux.native.close()?;
    drop(mux);
    util::check(&cancelled)?;
    let video_frames = routes
        .iter()
        .map(|r| {
            if let RouteMode::Video(stage) = &r.mode {
                stage.frames
            } else {
                0
            }
        })
        .sum();
    let audio_samples = routes
        .iter()
        .map(|r| {
            if let RouteMode::Audio(stage) = &r.mode {
                stage.samples
            } else {
                0
            }
        })
        .sum();
    Ok(Completed {
        temporary,
        target: target.path.clone(),
        result: MediaJobResult {
            outputs: vec![target.path.clone()],
            video_frames,
            audio_samples,
            processed: progress.processed,
        },
        progress,
    })
}
fn encoder_tag(name: &str) -> String {
    let version = codec::version();
    format!(
        "Lavc{}.{}.{} {name}",
        version >> 16,
        (version >> 8) & 255,
        version & 255
    )
}
