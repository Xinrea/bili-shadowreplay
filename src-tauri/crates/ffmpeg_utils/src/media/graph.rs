//! Multi-input demand scheduler and shared complex filter graph. All native
//! contexts and scratch leases live on one blocking worker; only packet/frame
//! references cross the small bounded startup queues.
use super::{
    audio::AudioStage,
    audio_encoder::AudioEncoderStage,
    decode::{DecodedFrame, GraphDecoder},
    execute::{self, Progress},
    raw_video::RawVideoReader,
    util::{self, Device, Limits},
    video::VideoStage,
    video_encoder::VideoEncoderStage,
    *,
};
use crate::profiling as perf;
use crate::{
    audio::{copy_chapters, input_metadata, remux::AudioPacketClock},
    metadata::run_media,
    output::NativeOutput,
};
use ffmpeg_next::{self as ffmpeg, codec, ffi, filter, format, frame, media, Packet, Rational};
use std::{
    collections::{BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};
use tempfile::NamedTempFile;

struct LiveGuard(Vec<RawVideoCancelHandle>);
impl Drop for LiveGuard {
    fn drop(&mut self) {
        for h in &self.0 {
            h.close();
        }
    }
}
struct Completed {
    files: Vec<(NamedTempFile, PathBuf)>,
    result: MediaJobResult,
    progress: Progress,
    _resources: Vec<MediaResourceLease>,
}
pub(super) async fn run(
    plan: MediaPlan,
    sources: MediaBindings,
    progress: Option<MediaProgressCallback>,
) -> Result<MediaJobResult, String> {
    // This guard lives outside the worker: dropping a queued future must also
    // release a producer blocked on a full BGRA queue immediately.
    let _live = LiveGuard(
        sources
            .raw_video
            .iter()
            .map(|(_, s)| s.cancel_handle())
            .collect(),
    );
    let mut completed = run_media(move |cancelled| {
        let _session = sources.profiler.as_ref().map(|profile| profile.start());
        crate::profiling::measure("worker", || {
            validate(&plan, &sources)?;
            let job = crate::profiling::measure("setup", || {
                Job::new(plan, sources, progress, cancelled)
            })?;
            job.run()
        })
    })
    .await?;
    // Every encoder/AVIO is closed before publishing any output. Publication
    // errors are reported; callers needing a cache pair retain their .part files.
    for (temp, target) in &completed.files {
        std::fs::metadata(temp.path()).map_err(|e| util::error("待发布媒体临时文件不可用", e))?;
        if target.is_dir() {
            return Err("媒体输出不能覆盖目录".into());
        }
        let parent = target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        if !parent.is_dir() {
            return Err("媒体输出目录已不存在".into());
        }
    }
    for (temp, target) in completed.files {
        temp.persist(&target)
            .map_err(|e| util::error("发布媒体输出失败", e.error))?;
    }
    completed.progress.emit(MediaStage::Completed, true);
    Ok(completed.result)
}
fn input_source(route: &StreamRoute) -> Option<(usize, StreamSelector)> {
    match route.source {
        StreamSource::Input { input, selector } => Some((input, selector)),
        _ => None,
    }
}
fn validate(plan: &MediaPlan, sources: &MediaBindings) -> Result<(), String> {
    if matches!(plan.progress_total, MediaProgressTotal::Known(value) if !value.is_finite() || value < 0.0)
    {
        return Err("媒体进度总时长无效".into());
    }
    crate::metadata::initialize()?;
    if plan.inputs.is_empty() || plan.outputs.is_empty() || plan.routes.is_empty() {
        return Err("媒体计划必须包含输入、输出和路由".into());
    }
    let mut paths = BTreeSet::new();
    for output in &plan.outputs {
        if !paths.insert(output.path.clone()) {
            return Err("多个媒体输出不能使用同一路径".into());
        }
        execute::format_name(output)?;
        util::options(&output.options)?;
        for t in [output.seek, output.duration].into_iter().flatten() {
            util::time(t)?;
        }
        for input in &plan.inputs {
            if let MediaSource::File(path) = &input.source {
                if *path == output.path
                    || path
                        .canonicalize()
                        .ok()
                        .zip(output.path.canonicalize().ok())
                        .is_some_and(|(a, b)| a == b)
                {
                    return Err("媒体输出不能覆盖输入".into());
                }
            }
        }
    }
    for input in &plan.inputs {
        util::options(&input.options)?;
        util::options(&input.decoder_options)?;
        if let Some(s) = input.seek {
            util::time(s.seconds)?;
        }
        if let Some(t) = input.duration {
            util::time(t)?;
        }
        if input
            .decoder_threads
            .is_some_and(|t| t == 0 || t > i32::MAX as usize)
        {
            return Err("解码线程数无效".into());
        }
        if !matches!(input.source, MediaSource::File(_))
            && (input.seek.is_some()
                || input.copy_timestamps
                || input.format.is_some()
                || !input.options.is_empty()
                || !input.decoder_options.is_empty()
                || input.decoder_threads.is_some())
        {
            return Err("生成/原始帧输入不支持文件定位或解码选项".into());
        }
        match &input.source {
            MediaSource::GeneratedVideo(s) => {
                if s.color.is_empty()
                    || s.color
                        .bytes()
                        .any(|c| matches!(c, 0 | b':' | b'\'' | b'[' | b']' | b';'))
                    || s.width == 0
                    || s.height == 0
                    || s.width > i32::MAX as u32
                    || s.height > i32::MAX as u32
                    || s.frame_rate_num == 0
                    || s.frame_rate_den == 0
                    || s.frame_rate_num > i32::MAX as u32
                    || s.frame_rate_den > i32::MAX as u32
                {
                    return Err("生成视频参数无效".into());
                }
            }
            MediaSource::GeneratedAudio(s) => {
                if s.sample_rate == 0
                    || s.sample_rate > i32::MAX as u32
                    || s.channel_layout.is_empty()
                    || s.channel_layout
                        .bytes()
                        .any(|c| matches!(c, 0 | b':' | b'\'' | b'[' | b']' | b';'))
                {
                    return Err("生成音频参数无效".into());
                }
            }
            MediaSource::RawVideo(s) => {
                s.validate()?;
            }
            MediaSource::File(_) => (),
        }
    }
    let mut bound = BTreeSet::new();
    for (i, s) in &sources.raw_video {
        if !bound.insert(*i) {
            return Err("原始视频输入重复绑定".into());
        }
        match plan.inputs.get(*i).map(|s| &s.source) {
            Some(MediaSource::RawVideo(spec)) if *spec == s.spec() => (),
            _ => return Err("原始视频绑定与声明不一致".into()),
        }
    }
    for (i, s) in plan.inputs.iter().enumerate() {
        if matches!(s.source, MediaSource::RawVideo(_)) && !bound.contains(&i) {
            return Err(format!("原始视频输入{i}没有绑定生产者"));
        }
    }
    let mut processed = BTreeSet::new();
    for (index, _) in &sources.video_processors {
        if !processed.insert(*index) {
            return Err("视频帧处理器重复绑定".into());
        }
        if !matches!(
            plan.routes.get(*index),
            Some(StreamRoute {
                source: StreamSource::Filter { .. },
                mode: StreamMode::Video(VideoEncodeOptions {
                    frame_processor: true,
                    ..
                }),
                ..
            })
        ) {
            return Err("视频帧处理器必须绑定已声明处理器的滤镜视频路由".into());
        }
    }
    for (index, route) in plan.routes.iter().enumerate() {
        if route.output >= plan.outputs.len() {
            return Err("媒体路由引用了不存在的输出".into());
        }
        if input_source(route).is_some_and(|(i, _)| i >= plan.inputs.len()) {
            return Err("媒体路由引用了不存在的输入".into());
        }
        match &route.mode {
            StreamMode::Video(o) => {
                execute::validate_video(o)?;
                if o.frame_processor && !processed.contains(&index) {
                    return Err("视频帧处理器缺少绑定".into());
                }
                if o.frame_processor && o.hardware_device.is_some() {
                    return Err("视频帧处理器自行管理硬件设备，不能再指定图设备".into());
                }
            }
            StreamMode::Audio(o) => {
                let c = ffmpeg::encoder::find_by_name(&o.encoder)
                    .ok_or_else(|| format!("找不到音频编码器 {}", o.encoder))?;
                if c.medium() != media::Type::Audio {
                    return Err("音频路由必须使用音频编码器".into());
                }
                util::validate_options(c, &o.options)?;
                if o.bit_rate.is_some_and(|v| v > i64::MAX as u64)
                    || o.sample_rate.is_some_and(|v| v == 0 || v > i32::MAX as u32)
                {
                    return Err("音频编码参数超出范围".into());
                }
            }
            StreamMode::Subtitle(_) => {
                return Err("多输入媒体计划尚不支持字幕路由；请显式使用单输入字幕编码计划".into())
            }
            StreamMode::Copy => (),
        }
    }
    if let Some(g) = &plan.filter_graph {
        if g.spec.trim().is_empty() || g.spec.as_bytes().contains(&0) {
            return Err("复杂滤镜图不能为空或包含NUL".into());
        }
        let mut names = BTreeSet::new();
        for b in &g.inputs {
            if matches!(b.selector, StreamSelector::AllAudio) {
                return Err("滤镜输入只能绑定单一音轨；AllAudio仅用于直接流映射".into());
            }
            label(&b.label)?;
            if !names.insert(b.label.clone()) || b.input >= plan.inputs.len() {
                return Err("滤镜图输入标签重复或输入索引无效".into());
            }
        }
        let mut outs = BTreeSet::new();
        for b in &g.outputs {
            label(&b.label)?;
            if !outs.insert(b.label.clone()) {
                return Err("滤镜图输出标签重复".into());
            }
        }
        for out in &g.outputs {
            let routes = plan
                .routes
                .iter()
                .filter(|r| matches!(&r.source,StreamSource::Filter{label} if *label==out.label))
                .collect::<Vec<_>>();
            if routes.len() != 1 {
                return Err(format!(
                    "滤镜图输出{}必须且只能映射一次；多路输出请在图内使用split/asplit",
                    out.label
                ));
            }
            if !matches!(
                (&routes[0].mode, out.kind),
                (StreamMode::Video(_), MediaKind::Video) | (StreamMode::Audio(_), MediaKind::Audio)
            ) {
                return Err("滤镜输出类型必须匹配视频/音频编码路由".into());
            }
        }
        for r in &plan.routes {
            if let StreamSource::Filter { label } = &r.source {
                if !outs.contains(label) {
                    return Err(format!("滤镜路由引用了未声明的输出{label}"));
                }
            }
        }
    } else if plan
        .routes
        .iter()
        .any(|r| matches!(r.source, StreamSource::Filter { .. }))
    {
        return Err("滤镜输出路由缺少滤镜图".into());
    }
    Ok(())
}
fn label(value: &str) -> Result<(), String> {
    if value.is_empty() || value.bytes().any(|v| matches!(v, 0 | b'[' | b']' | b';')) {
        Err("滤镜标签无效".into())
    } else {
        Ok(())
    }
}

struct FileInput {
    context: format::context::Input,
    offset: i64,
    seek_target: i64,
    eof: bool,
}
impl FileInput {
    fn new(source: &MediaInput, cancelled: Arc<AtomicBool>) -> Result<Self, String> {
        let mut context = super::input::open(source, cancelled)?;
        let origin = unsafe { (*context.as_ptr()).start_time };
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
            let preroll =
                unsafe { (*context.format().as_ptr()).flags & ffi::AVFMT_SEEK_TO_PTS == 0 }
                    && context
                        .streams()
                        .any(|s| unsafe { (*s.parameters().as_ptr()).video_delay > 0 });
            let seek = if preroll {
                seek_target
                    .checked_sub(3 * i64::from(ffi::AV_TIME_BASE) / 23)
                    .ok_or("媒体定位时间溢出")?
            } else {
                seek_target
            };
            let _seek_result = context.seek(seek, ..seek);
        }
        let offset = if source.copy_timestamps {
            0
        } else {
            seek_target.checked_neg().ok_or("媒体时间戳溢出")?
        };
        Ok(Self {
            context,
            offset,
            seek_target,
            eof: false,
        })
    }
    fn limits(&self, source: &MediaInput, output: &MediaOutput) -> Result<Limits, String> {
        Ok(Limits {
            input_start: source.seek.filter(|s| s.accurate).map(|_| {
                if source.copy_timestamps {
                    self.seek_target
                } else {
                    0
                }
            }),
            input_duration: source.duration.map(util::time).transpose()?,
            output_start: output.seek.map(util::time).transpose()?.unwrap_or(0),
            output_seek: output.seek.is_some(),
            output_duration: output.duration.map(util::time).transpose()?,
        })
    }
}
struct CopyState {
    parameters: codec::Parameters,
    clock: AudioPacketClock,
    packet_clock: super::packet_clock::PacketClock,
    medium: media::Type,
    started: bool,
    done: bool,
    limits: Limits,
    input_end: Option<i64>,
}
enum Mode {
    Copy(Box<CopyState>),
    FileVideo(Box<VideoStage>),
    FileAudio(Box<AudioStage>),
    Video(Box<VideoEncoderStage>),
    Audio(Box<AudioEncoderStage>),
}
struct Route {
    label: Option<String>,
    source: Option<(usize, usize)>,
    output: usize,
    stream: usize,
    base: Rational,
    metadata: ffmpeg::Dictionary<'static>,
    mode: Mode,
    finished: bool,
}
impl Route {
    fn ready(&self) -> bool {
        match &self.mode {
            Mode::Copy(_) => true,
            Mode::FileVideo(s) => s.ready(),
            Mode::FileAudio(s) => s.ready(),
            Mode::Video(s) => s.ready(),
            Mode::Audio(s) => s.ready(),
        }
    }
    fn done(&self) -> bool {
        match &self.mode {
            Mode::Copy(s) => s.done,
            Mode::FileVideo(s) => s.done(),
            Mode::FileAudio(s) => s.done(),
            Mode::Video(s) => s.done(),
            Mode::Audio(s) => s.done(),
        }
    }
    fn params(&self) -> Option<codec::Parameters> {
        match &self.mode {
            Mode::Copy(s) => Some(s.parameters.clone()),
            Mode::FileVideo(s) => s.parameters(),
            Mode::FileAudio(s) => s.parameters(),
            Mode::Video(s) => s.parameters(),
            Mode::Audio(s) => s.parameters(),
        }
    }
    fn base(&self) -> Rational {
        match &self.mode {
            Mode::Copy(_) => self.base,
            Mode::FileVideo(s) => s.encoder_base(),
            Mode::FileAudio(s) => s.encoder_base(),
            Mode::Video(s) => s.encoder_base(),
            Mode::Audio(s) => s.encoder_base(),
        }
    }
    fn rate(&self) -> Option<Rational> {
        match &self.mode {
            Mode::FileVideo(s) => Some(s.frame_rate()),
            Mode::Video(s) => Some(s.frame_rate()),
            _ => None,
        }
    }
}
struct Output {
    native: NativeOutput,
    temp: Option<NamedTempFile>,
    target: PathBuf,
    header: bool,
    pending: VecDeque<(usize, Packet, Rational, bool)>,
    pending_bytes: usize,
    last: Vec<Option<i64>>,
    flags: i32,
    global: bool,
    options: ffmpeg::Dictionary<'static>,
}
impl Output {
    fn new(spec: &MediaOutput, cancelled: Arc<AtomicBool>) -> Result<Self, String> {
        let name = execute::format_name(spec)?;
        let parent = spec
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let suffix = spec
            .path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| format!(".{s}"))
            .unwrap_or_else(|| ".tmp".into());
        let temp = tempfile::Builder::new()
            .prefix(".media-job-")
            .suffix(&suffix)
            .tempfile_in(parent)
            .map_err(|e| util::error("创建媒体临时输出失败", e))?;
        let mut native = NativeOutput::new(temp.path(), &name, cancelled)?;
        let flags = unsafe { (*native.output.format().as_ptr()).flags };
        let global = native
            .output
            .format()
            .flags()
            .contains(format::flag::Flags::GLOBAL_HEADER);
        unsafe {
            (*native.output.as_mut_ptr()).max_delay = 700_000;
            (*native.output.as_mut_ptr()).avoid_negative_ts = match spec.negative_ts {
                NegativeTs::Auto => ffi::AVFMT_AVOID_NEG_TS_AUTO,
                NegativeTs::Disabled => ffi::AVFMT_AVOID_NEG_TS_DISABLED,
                NegativeTs::MakeZero => ffi::AVFMT_AVOID_NEG_TS_MAKE_ZERO,
            };
        }
        let mut opts = spec.options.clone();
        if spec.faststart {
            opts.entry("movflags".into())
                .and_modify(|v| v.push_str("+faststart"))
                .or_insert_with(|| "+faststart".into());
        }
        Ok(Self {
            native,
            temp: Some(temp),
            target: spec.path.clone(),
            header: false,
            pending: VecDeque::new(),
            pending_bytes: 0,
            last: Vec::new(),
            flags,
            global,
            options: util::options(&opts)?,
        })
    }
    #[allow(clippy::too_many_arguments)]
    fn emit(
        &mut self,
        route: usize,
        stream: usize,
        mut packet: Packet,
        base: Rational,
        copy: bool,
        progress: &mut Progress,
        cancelled: &AtomicBool,
    ) -> Result<(), String> {
        util::check(cancelled)?;
        if !self.header || copy {
            self.pending_bytes = self
                .pending_bytes
                .checked_add(packet.size())
                .ok_or("媒体启动队列溢出")?;
            self.pending.push_back((route, packet, base, copy));
            if self.pending_bytes > 64 * 1024 * 1024 || (!self.header && self.pending.len() > 1024)
            {
                return Err("媒体流参数尚未齐备，启动复用队列超出上限".into());
            }
            return Ok(());
        }
        let output_base = self
            .native
            .output
            .stream(stream)
            .ok_or("媒体输出流不存在")?
            .time_base();
        packet.rescale_ts(base, output_base);
        self.write(stream, packet, progress, cancelled)
    }
    fn write(
        &mut self,
        stream: usize,
        mut packet: Packet,
        progress: &mut Progress,
        cancelled: &AtomicBool,
    ) -> Result<(), String> {
        util::check(cancelled)?;
        let base = self
            .native
            .output
            .stream(stream)
            .ok_or("媒体输出流不存在")?
            .time_base();
        packet.set_stream(stream);
        packet.set_position(-1);
        if self.flags & ffi::AVFMT_NOTIMESTAMPS == 0 {
            if let (Some(dts), Some(last)) = (packet.dts(), self.last[stream]) {
                let minimum = last
                    .checked_add(i64::from(self.flags & ffi::AVFMT_TS_NONSTRICT == 0))
                    .ok_or("媒体DTS溢出")?;
                if dts < minimum {
                    if packet.pts().is_some_and(|p| p >= dts) {
                        packet.set_pts(Some(packet.pts().unwrap().max(minimum)));
                    }
                    packet.set_dts(Some(minimum));
                }
            }
        }
        self.last[stream] = packet.dts();
        progress.packet(&packet, base);
        perf::measure("mux_write", || {
            packet.write_interleaved(&mut self.native.output)
        })
        .map_err(|e| util::error("写入多输入媒体包失败", e))
    }
}
struct Binding {
    input: usize,
    decoder: Option<usize>,
    raw: Option<RawVideoReader>,
    template: Option<DecodedFrame>,
    rate: Rational,
    base: Rational,
    rotation: Option<[i32; 9]>,
    eof: bool,
    flushed: bool,
}
struct Graph {
    graph: filter::Graph,
    sinks: Vec<(usize, String)>,
    sources: Vec<String>,
    finished: Vec<bool>,
}
struct Job {
    files: Vec<Option<FileInput>>,
    outputs: Vec<Output>,
    routes: Vec<Route>,
    decoders: Vec<GraphDecoder>,
    bindings: Vec<Binding>,
    graph: Option<Graph>,
    device: Option<Device>,
    progress: Progress,
    cancelled: Arc<AtomicBool>,
    // Contexts and temporary outputs must close before scratch resources.
    plan: MediaPlan,
}
impl Job {
    fn new(
        mut plan: MediaPlan,
        sources: MediaBindings,
        callback: Option<MediaProgressCallback>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        // Bind by the filter label before AllAudio expands direct routes, so
        // application route indices retain their declared meaning.
        let mut processors = std::collections::BTreeMap::new();
        for (index, factory) in sources.video_processors {
            let Some(StreamRoute {
                source: StreamSource::Filter { label },
                ..
            }) = plan.routes.get(index)
            else {
                return Err("视频帧处理器路由不存在".into());
            };
            processors.insert(label.clone(), factory);
        }
        let mut files = Vec::new();
        for input in &plan.inputs {
            files.push(if matches!(input.source, MediaSource::File(_)) {
                Some(FileInput::new(input, cancelled.clone())?)
            } else {
                None
            });
        }
        let contexts = files
            .iter()
            .map(|f| f.as_ref().map(|f| &f.context))
            .collect::<Vec<_>>();
        let total = super::input::progress_total(&plan, &contexts)?;
        plan.routes = super::input::expand_routes(&plan.routes, &contexts)?;
        let mut outputs = plan
            .outputs
            .iter()
            .map(|s| Output::new(s, cancelled.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        let mut routes = Vec::new();
        for route in &plan.routes {
            let target = &plan.outputs[route.output];
            let output = &mut outputs[route.output];
            let mut metadata = ffmpeg::Dictionary::new();
            let mut base = Rational(1, ffi::AV_TIME_BASE);
            let limits = Limits {
                input_start: None,
                input_duration: None,
                output_start: target.seek.map(util::time).transpose()?.unwrap_or(0),
                output_seek: target.seek.is_some(),
                output_duration: target.duration.map(util::time).transpose()?,
            };
            let (source, mode) = match &route.source {
                StreamSource::Filter { label } => {
                    let mode = match &route.mode {
                        StreamMode::Video(o) => {
                            metadata.set("encoder", &execute::encoder_tag(&o.encoder));
                            Mode::Video(Box::new(
                                VideoEncoderStage::new(
                                    o.clone(),
                                    limits,
                                    output.global,
                                    output.flags,
                                    plan.inputs.iter().any(|i| i.copy_timestamps),
                                    execute::format_name(target)? == "avi",
                                    cancelled.clone(),
                                )?
                                .with_processor_factory(processors.remove(label)),
                            ))
                        }
                        StreamMode::Audio(o) => {
                            metadata.set("encoder", &execute::encoder_tag(&o.encoder));
                            Mode::Audio(Box::new(AudioEncoderStage::new(
                                o.clone(),
                                limits,
                                output.global,
                                cancelled.clone(),
                            )?))
                        }
                        _ => return Err("复杂滤镜输出必须显式编码".into()),
                    };
                    (None, mode)
                }
                StreamSource::Input { input, selector } => {
                    let file = files[*input]
                        .as_ref()
                        .ok_or("生成/原始帧输入必须通过滤镜图输出")?;
                    let Some(index) =
                        execute::selected(&file.context, *selector, codec::Id::None, false)
                    else {
                        if route.required {
                            return Err(format!("输入{input}没有匹配流 {selector:?}"));
                        }
                        continue;
                    };
                    let stream = file.context.stream(index).ok_or("媒体输入流不存在")?;
                    base = stream.time_base();
                    let parameters = stream.parameters();
                    let (_, tags) = input_metadata(&file.context, index)?;
                    metadata = tags;
                    let source = &plan.inputs[*input];
                    let limits = file.limits(source, target)?;
                    let mode = match &route.mode {
                        StreamMode::Copy => {
                            let input_end = source
                                .duration
                                .map(util::time)
                                .transpose()?
                                .map(|d| {
                                    (if source.copy_timestamps {
                                        file.seek_target
                                    } else {
                                        0
                                    })
                                    .checked_add(d)
                                    .ok_or("输入媒体复制范围溢出")
                                })
                                .transpose()?;
                            Mode::Copy(Box::new(CopyState {
                                parameters: parameters.clone(),
                                clock: AudioPacketClock::new(),
                                packet_clock: super::packet_clock::PacketClock::new(&stream),
                                medium: parameters.medium(),
                                started: false,
                                done: false,
                                limits,
                                input_end,
                            }))
                        }
                        StreamMode::Audio(o) => {
                            if parameters.medium() != media::Type::Audio {
                                return Err("音频路由选择了非音频流".into());
                            }
                            metadata.set("encoder", &execute::encoder_tag(&o.encoder));
                            Mode::FileAudio(Box::new(AudioStage::new(
                                parameters,
                                source,
                                base,
                                o.clone(),
                                limits,
                                output.global,
                                cancelled.clone(),
                            )?))
                        }
                        StreamMode::Video(o) => {
                            if parameters.medium() != media::Type::Video {
                                return Err("视频路由选择了非视频流".into());
                            }
                            metadata.set("encoder", &execute::encoder_tag(&o.encoder));
                            let rotation = stream
                                .side_data()
                                .find(|s| {
                                    s.kind() == ffmpeg::packet::side_data::Type::DisplayMatrix
                                })
                                .and_then(|s| crate::video_frame::display_matrix(s.data()));
                            let rate = unsafe {
                                ffi::av_guess_frame_rate(
                                    file.context.as_ptr().cast_mut(),
                                    stream.as_ptr().cast_mut(),
                                    std::ptr::null_mut(),
                                )
                            };
                            Mode::FileVideo(Box::new(VideoStage::new(
                                parameters,
                                source,
                                base,
                                rate.into(),
                                stream.avg_frame_rate(),
                                rotation,
                                o.clone(),
                                limits,
                                output.global,
                                output.flags,
                                file.context.nb_streams() == 1 || source.copy_timestamps,
                                execute::format_name(target)? == "avi",
                                unsafe {
                                    (*file.context.format().as_ptr()).flags
                                        & ffi::AVFMT_NOTIMESTAMPS
                                        != 0
                                },
                                cancelled.clone(),
                            )?))
                        }
                        StreamMode::Subtitle(_) => return Err("多输入字幕路由尚未实现".into()),
                    };
                    (Some((*input, index)), mode)
                }
            };
            let mut stream = output
                .native
                .output
                .add_stream(ffmpeg::encoder::find(codec::Id::None))
                .map_err(|e| util::error("创建多输入媒体输出流失败", e))?;
            let stream_index = stream.index();
            stream.set_time_base(base);
            if let Mode::Copy(copy) = &mode {
                stream.set_parameters(copy.parameters.clone());
            }
            output.last.push(None);
            routes.push(Route {
                label: match &route.source {
                    StreamSource::Filter { label } => Some(label.clone()),
                    _ => None,
                },
                source,
                output: route.output,
                stream: stream_index,
                base,
                metadata,
                mode,
                finished: false,
            });
        }
        for (index, output) in outputs.iter_mut().enumerate() {
            if !routes.iter().any(|r| r.output == index) {
                return Err(format!("输出{index}没有选中任何媒体流"));
            }
            if let Some((file, _)) = files
                .iter()
                .filter_map(Option::as_ref)
                .find_map(|f| f.context.streams().next().map(|s| (f, s.index())))
            {
                let (global, _) = input_metadata(&file.context, 0)?;
                output.native.output.set_metadata(global);
                let offset = plan.outputs[index]
                    .seek
                    .map(util::time)
                    .transpose()?
                    .unwrap_or(0)
                    .checked_sub(file.offset)
                    .ok_or("媒体章节偏移溢出")?;
                copy_chapters(&file.context, &mut output.native.output, offset)?;
            }
        }
        if !processors.is_empty() {
            return Err("视频帧处理器未连接至滤镜图".into());
        }
        let mut decoders = Vec::new();
        let mut bindings = Vec::new();
        let mut raw = sources
            .raw_video
            .into_iter()
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut used_streams = BTreeSet::new();
        if let Some(graph) = &plan.filter_graph {
            for binding in &graph.inputs {
                let input = &plan.inputs[binding.input];
                let mut state = Binding {
                    input: binding.input,
                    decoder: None,
                    raw: None,
                    template: None,
                    rate: Rational(0, 1),
                    base: Rational(1, ffi::AV_TIME_BASE),
                    rotation: None,
                    eof: false,
                    flushed: false,
                };
                match &input.source {
                    MediaSource::File(_) => {
                        let context = &files[binding.input].as_ref().unwrap().context;
                        let stream =
                            execute::selected(context, binding.selector, codec::Id::None, false)
                                .and_then(|i| context.stream(i))
                                .ok_or_else(|| {
                                    format!(
                                        "滤镜输入{}没有匹配流 {:?}",
                                        binding.label, binding.selector
                                    )
                                })?;
                        if !used_streams.insert((binding.input, stream.index())) {
                            return Err(
                                "同一媒体流只能绑定一个滤镜输入；请在图中使用split/asplit".into()
                            );
                        }
                        let decoder = GraphDecoder::new(binding.input, &stream, input, context)?;
                        state.base = decoder.base;
                        state.rate = decoder.rate;
                        state.rotation = decoder.rotation;
                        state.decoder = Some(decoders.len());
                        decoders.push(decoder);
                    }
                    MediaSource::RawVideo(spec) => {
                        if !matches!(
                            binding.selector,
                            StreamSelector::FirstVideo
                                | StreamSelector::AutoVideo
                                | StreamSelector::Index(0)
                        ) {
                            return Err("原始视频输入只能选择视频0".into());
                        }
                        state.template = Some(DecodedFrame::Video(spec.template()?));
                        state.rate = spec.rate();
                        state.base = spec.time_base();
                        state.raw = Some(
                            raw.remove(&binding.input)
                                .ok_or("原始视频缺少绑定")?
                                .into_reader(),
                        );
                    }
                    MediaSource::GeneratedVideo(s) => {
                        if !matches!(
                            binding.selector,
                            StreamSelector::FirstVideo
                                | StreamSelector::AutoVideo
                                | StreamSelector::Index(0)
                        ) {
                            return Err("生成视频输入只能选择视频0".into());
                        }
                        state.rate = Rational(s.frame_rate_num as i32, s.frame_rate_den as i32);
                        state.base = state.rate.invert();
                    }
                    MediaSource::GeneratedAudio(_) => {
                        if !matches!(
                            binding.selector,
                            StreamSelector::FirstAudio
                                | StreamSelector::AutoAudio
                                | StreamSelector::Index(0)
                        ) {
                            return Err("生成音频输入只能选择音频0".into());
                        }
                    }
                }
                bindings.push(state);
            }
        }
        if !raw.is_empty() {
            return Err("原始视频绑定未连接至滤镜图".into());
        }
        let mut device = None;
        let mut device_key = None;
        for route in &plan.routes {
            if let StreamMode::Video(o) = &route.mode {
                if !matches!(route.source, StreamSource::Filter { .. }) {
                    continue;
                }
                if let Some(d) = &o.hardware_device {
                    let key = (d.kind.clone(), d.device.clone());
                    if device_key.as_ref().is_some_and(|k| *k != key) {
                        return Err("同一共享滤镜图不能混用不同硬件设备".into());
                    }
                    if device_key.is_none() {
                        device = Some(Device::new(d)?);
                        device_key = Some(key);
                    }
                }
            }
        }
        let mut progress = Progress::new(callback, total);
        progress.emit(MediaStage::Processing, true);
        Ok(Self {
            plan,
            files,
            outputs,
            routes,
            decoders,
            bindings,
            graph: None,
            device,
            progress,
            cancelled,
        })
    }
    fn configure_graph(&mut self) -> Result<(), String> {
        let spec = self.plan.filter_graph.as_ref().ok_or("没有复杂滤镜图")?;
        let mut graph = filter::Graph::new();
        unsafe {
            (*graph.as_mut_ptr()).nb_threads = 1;
        }
        let mut parts = Vec::new();
        let mut sources = Vec::new();
        for (index, binding) in self.bindings.iter().enumerate() {
            let source = &self.plan.inputs[binding.input];
            let name = format!("_native_source_{index}");
            let mut filters = Vec::new();
            match &source.source {
                MediaSource::File(_) | MediaSource::RawVideo(_) => {
                    let template = binding.template.as_ref().ok_or("图输入没有可用的格式帧")?;
                    let kind = match template {
                        DecodedFrame::Audio(_) => "abuffer",
                        DecodedFrame::Video(_) => "buffer",
                    };
                    graph
                        .add(
                            &filter::find(kind).ok_or_else(|| format!("缺少{kind}滤镜"))?,
                            &name,
                            &template.buffer_args(binding.base, binding.rate)?,
                        )
                        .map_err(|e| util::error("创建共享图输入失败", e))?;
                    if let DecodedFrame::Video(frame) = template {
                        let matrix = frame
                            .side_data(frame::side_data::Type::DisplayMatrix)
                            .and_then(|s| crate::video_frame::display_matrix(s.data()))
                            .or(binding.rotation);
                        filters.extend(crate::video_frame::rotation_filters(matrix));
                    }
                    if let Some(file) = &self.files[binding.input] {
                        let limits = file.limits(source, &MediaOutput::file(PathBuf::new()))?;
                        if let Some(trim) =
                            limits.trim(matches!(template, DecodedFrame::Audio(_)), true)
                        {
                            filters.push(trim);
                        }
                    } else if let Some(duration) = source.duration {
                        let limits = Limits {
                            input_start: None,
                            input_duration: Some(util::time(duration)?),
                            output_start: 0,
                            output_seek: false,
                            output_duration: None,
                        };
                        if let Some(trim) = limits.trim(false, true) {
                            filters.push(trim);
                        }
                    }
                }
                MediaSource::GeneratedVideo(s) => {
                    let mut args = format!(
                        "c={}:s={}x{}:r={}/{}",
                        s.color, s.width, s.height, s.frame_rate_num, s.frame_rate_den
                    );
                    if let Some(duration) = source.duration {
                        args.push_str(&format!(":d={}us", util::time(duration)?));
                    }
                    graph
                        .add(&filter::find("color").ok_or("缺少color滤镜")?, &name, &args)
                        .map_err(|e| util::error("创建生成视频源失败", e))?;
                }
                MediaSource::GeneratedAudio(s) => {
                    let mut args = format!("r={}:cl={}", s.sample_rate, s.channel_layout);
                    if let Some(duration) = source.duration {
                        args.push_str(&format!(":d={}us", util::time(duration)?));
                    }
                    graph
                        .add(
                            &filter::find("anullsrc").ok_or("缺少anullsrc滤镜")?,
                            &name,
                            &args,
                        )
                        .map_err(|e| util::error("创建静音源失败", e))?;
                }
            }
            if filters.is_empty() {
                filters.push(
                    if matches!(source.source, MediaSource::GeneratedAudio(_))
                        || matches!(binding.template, Some(DecodedFrame::Audio(_)))
                    {
                        "anull".into()
                    } else {
                        "null".into()
                    },
                );
            }
            parts.push(format!(
                "[{name}]{}[{}]",
                filters.join(","),
                spec.inputs[index].label
            ));
            sources.push(name);
        }
        // CLI permits a trailing chain separator; joining our output chains
        // must not turn that into an empty `;;` chain. Keep an escaped literal
        // semicolon in a final filter argument intact.
        let script = spec
            .spec
            .trim_end_matches(|c: char| c.is_ascii_whitespace());
        let backslashes = script
            .strip_suffix(';')
            .map(|s| s.bytes().rev().take_while(|c| *c == b'\\').count());
        let script = if backslashes.is_some_and(|n| n % 2 == 0) {
            &script[..script.len() - 1]
        } else {
            script
        };
        parts.push(script.to_owned());
        let mut sinks = Vec::new();
        for output in &spec.outputs {
            let route = self
                .plan
                .routes
                .iter()
                .find(|r| matches!(&r.source,StreamSource::Filter{label} if *label==output.label))
                .ok_or("图输出缺少路由")?;
            let route_index = self
                .routes
                .iter()
                .position(|r| r.label.as_ref() == Some(&output.label))
                .ok_or("图输出路由不存在")?;
            let name = format!("_native_sink_{route_index}");
            let mut filters = Vec::new();
            let limits = Limits {
                input_start: None,
                input_duration: None,
                output_start: self.plan.outputs[route.output]
                    .seek
                    .map(util::time)
                    .transpose()?
                    .unwrap_or(0),
                output_seek: self.plan.outputs[route.output].seek.is_some(),
                output_duration: self.plan.outputs[route.output]
                    .duration
                    .map(util::time)
                    .transpose()?,
            };
            match (&mut self.routes[route_index].mode, &route.mode) {
                (Mode::Video(enc), StreamMode::Video(options)) => {
                    if let Some(s) = options.filter.as_ref().filter(|s| !s.trim().is_empty()) {
                        filters.push(s.clone());
                    }
                    if self.device.is_none() {
                        if let Some((width, height)) = enc.size() {
                            filters.push(format!("scale={width}:{height}"));
                        }
                    }
                    filters.push(enc.format_filter()?);
                    if let Some(trim) = limits.trim(false, false) {
                        filters.push(trim);
                    }
                    graph
                        .add(
                            &filter::find("buffersink").ok_or("缺少buffersink滤镜")?,
                            &name,
                            "",
                        )
                        .map_err(|e| util::error("创建共享图视频输出失败", e))?;
                }
                (Mode::Audio(enc), StreamMode::Audio(options)) => {
                    if let Some(s) = options.filter.as_ref().filter(|s| !s.trim().is_empty()) {
                        filters.push(s.clone());
                    }
                    filters.push(enc.format_filter()?);
                    if let Some(trim) = limits.trim(true, false) {
                        filters.push(trim);
                    }
                    graph
                        .add(
                            &filter::find("abuffersink").ok_or("缺少abuffersink滤镜")?,
                            &name,
                            "",
                        )
                        .map_err(|e| util::error("创建共享图音频输出失败", e))?;
                }
                _ => return Err("滤镜输出路由类型不匹配".into()),
            }
            parts.push(format!("[{}]{}[{name}]", output.label, filters.join(",")));
            sinks.push((route_index, name));
        }
        let mut parser = filter::graph::Parser::new(&mut graph);
        for source in &sources {
            parser = parser
                .output(source, 0)
                .map_err(|e| util::error("连接共享图输入失败", e))?;
        }
        for (_, sink) in &sinks {
            parser = parser
                .input(sink, 0)
                .map_err(|e| util::error("连接共享图输出失败", e))?;
        }
        parser
            .parse(&parts.join(";"))
            .map_err(|e| util::error("解析共享滤镜图失败", e))?;
        if let Some(device) = &self.device {
            device.attach(&mut graph)?;
        }
        graph
            .validate()
            .map_err(|e| util::error("配置共享滤镜图失败", e))?;
        for (index, name) in &sinks {
            if let Mode::Audio(enc) = &mut self.routes[*index].mode {
                let mut sink = graph.get(name).ok_or("共享图输出不存在")?;
                enc.open_from_sink(&mut sink)?;
            }
        }
        for binding in &mut self.bindings {
            binding.flushed = false;
        }
        self.graph = Some(Graph {
            finished: vec![false; sinks.len()],
            graph,
            sinks,
            sources,
        });
        Ok(())
    }
    fn headers(&mut self) -> Result<(), String> {
        for (index, out) in self.outputs.iter_mut().enumerate() {
            if out.header
                || self
                    .routes
                    .iter()
                    .filter(|r| r.output == index)
                    .any(|r| !r.ready())
            {
                continue;
            }
            let tags = unsafe { (*out.native.output.format().as_ptr()).codec_tag };
            for route in self.routes.iter_mut().filter(|r| r.output == index) {
                let mut target = out
                    .native
                    .output
                    .stream_mut(route.stream)
                    .ok_or("媒体输出流不存在")?;
                target.set_parameters(route.params().ok_or("媒体路由没有编码参数")?);
                target.set_time_base(route.base());
                if let Some((input, stream)) = route.source {
                    let source = self.files[input]
                        .as_ref()
                        .unwrap()
                        .context
                        .stream(stream)
                        .ok_or("媒体输入流不存在")?;
                    let rotate = matches!(&route.mode,Mode::FileVideo(s) if s.autorotate());
                    util::side_data(&source, &mut target, rotate)?;
                    unsafe {
                        (*target.as_mut_ptr()).disposition = (*source.as_ptr()).disposition;
                    }
                    if matches!(route.mode, Mode::Copy(_)) {
                        target.set_rate(source.rate());
                        target.set_avg_frame_rate(source.avg_frame_rate());
                        unsafe {
                            let raw = &mut *target.as_mut_ptr();
                            let p = &mut *raw.codecpar;
                            let mut supported = 0;
                            if !tags.is_null()
                                && ffi::av_codec_get_id(tags, p.codec_tag) != p.codec_id
                                && ffi::av_codec_get_tag2(tags, p.codec_id, &mut supported) != 0
                            {
                                p.codec_tag = 0;
                            }
                            if p.codec_type == ffi::AVMediaType::AVMEDIA_TYPE_VIDEO {
                                let aspect = (*source.as_ptr()).sample_aspect_ratio;
                                if aspect.num != 0 {
                                    p.sample_aspect_ratio = aspect;
                                }
                                raw.sample_aspect_ratio = p.sample_aspect_ratio;
                            }
                            if p.codec_id == ffi::AVCodecID::AV_CODEC_ID_AC3
                                || (p.codec_id == ffi::AVCodecID::AV_CODEC_ID_MP3
                                    && matches!(p.block_align, 1 | 576 | 1152))
                            {
                                p.block_align = 0;
                            }
                        }
                    }
                }
                if let Some(rate) = route.rate() {
                    target.set_avg_frame_rate(rate);
                }
                target.set_metadata(std::mem::take(&mut route.metadata));
            }
            let options = std::mem::take(&mut out.options);
            let remaining = out
                .native
                .output
                .write_header_with(options)
                .map_err(|e| util::error("写入多输入媒体文件头失败", e))?;
            util::unused(&remaining, "封装器")?;
            out.header = true;
        }
        self.flush_pending()
    }
    fn flush_pending(&mut self) -> Result<(), String> {
        for out in &mut self.outputs {
            if !out.header {
                continue;
            }
            while let Some((index, mut packet, base, copy)) = out.pending.pop_front() {
                out.pending_bytes -= packet.size();
                let route = &mut self.routes[index];
                let output_base = out
                    .native
                    .output
                    .stream(route.stream)
                    .ok_or("输出流不存在")?
                    .time_base();
                if copy {
                    match &mut route.mode {
                        Mode::Copy(state) if state.medium == media::Type::Audio => state
                            .clock
                            .rescale(&mut packet, base, output_base, &state.parameters)?,
                        _ => packet.rescale_ts(base, output_base),
                    }
                } else {
                    packet.rescale_ts(base, output_base);
                }
                out.write(route.stream, packet, &mut self.progress, &self.cancelled)?;
            }
        }
        Ok(())
    }
    fn read_input(&mut self, index: usize) -> Result<bool, String> {
        util::check(&self.cancelled)?;
        let file = self.files[index].as_mut().ok_or("读取了非文件输入")?;
        if file.eof {
            return Ok(false);
        }
        let mut packet = Packet::empty();
        match perf::measure("demux_read", || packet.read(&mut file.context)) {
            Ok(()) => (),
            Err(ffmpeg::Error::Eof) => {
                file.eof = true;
                for decoder in self.decoders.iter_mut().filter(|d| d.input == index) {
                    decoder.end_input();
                }
                for route_index in 0..self.routes.len() {
                    if self.routes[route_index]
                        .source
                        .is_some_and(|(i, _)| i == index)
                    {
                        self.finish_route(route_index)?;
                    }
                }
                self.headers()?;
                return Ok(true);
            }
            Err(e) if util::again(e) => return Ok(true),
            Err(e) => return Err(util::error(&format!("读取媒体输入{index}失败"), e)),
        }
        let stream = packet.stream();
        let base = file
            .context
            .stream(stream)
            .ok_or("读取到不存在的媒体流")?
            .time_base();
        let offset = unsafe { ffi::av_rescale_q(file.offset, util::MICROS, base.into()) };
        if let Some(pts) = packet.pts() {
            packet.set_pts(Some(pts.checked_add(offset).ok_or("媒体PTS溢出")?));
        }
        if let Some(dts) = packet.dts() {
            packet.set_dts(Some(dts.checked_add(offset).ok_or("媒体DTS溢出")?));
        }
        for decoder in self
            .decoders
            .iter_mut()
            .filter(|d| d.input == index && d.stream == stream)
        {
            decoder.enqueue(&packet)?;
        }
        for route_index in 0..self.routes.len() {
            if self.routes[route_index].source != Some((index, stream))
                || self.routes[route_index].done()
            {
                continue;
            }
            let output_index = self.routes[route_index].output;
            let output_stream = self.routes[route_index].stream;
            let out = &mut self.outputs[output_index];
            let progress = &mut self.progress;
            let cancelled = &self.cancelled;
            let mut emit = |packet, base| {
                out.emit(
                    route_index,
                    output_stream,
                    packet,
                    base,
                    false,
                    progress,
                    cancelled,
                )
            };
            match &mut self.routes[route_index].mode {
                Mode::FileVideo(s) => s.packet(&packet, &mut emit)?,
                Mode::FileAudio(s) => s.packet(&packet, &mut emit)?,
                Mode::Copy(state) => {
                    let source = self.files[index]
                        .as_ref()
                        .unwrap()
                        .context
                        .stream(stream)
                        .ok_or("复制源流不存在")?;
                    let repeat = unsafe {
                        let parser = ffi::av_stream_get_parser(source.as_ptr().cast_mut());
                        (!parser.is_null()).then(|| (*parser).repeat_pict)
                    };
                    let timestamp =
                        state
                            .packet_clock
                            .update(&packet, base, &state.parameters, repeat)?;
                    let mut end = state
                        .limits
                        .output_duration
                        .map(|d| {
                            state
                                .limits
                                .output_start
                                .checked_add(d)
                                .ok_or("媒体复制范围溢出")
                        })
                        .transpose()?;
                    if let Some(input_end) = state.input_end {
                        end = Some(end.map_or(input_end, |e| e.min(input_end)));
                    }
                    if end.is_some_and(|end| timestamp >= end) {
                        state.done = true;
                        continue;
                    }
                    if !state.started
                        && ((state.limits.output_seek && timestamp < state.limits.output_start)
                            || !packet.is_key())
                    {
                        continue;
                    }
                    state.started = true;
                    let mut copy = packet.clone();
                    if copy.dts().is_none() {
                        copy.set_dts(Some(unsafe {
                            ffi::av_rescale_q(timestamp, util::MICROS, base.into())
                        }));
                    }
                    let offset = unsafe {
                        ffi::av_rescale_q(state.limits.output_start, util::MICROS, base.into())
                    };
                    if let Some(pts) = copy.pts() {
                        copy.set_pts(Some(pts.checked_sub(offset).ok_or("媒体PTS溢出")?));
                    }
                    if let Some(dts) = copy.dts() {
                        copy.set_dts(Some(dts.checked_sub(offset).ok_or("媒体DTS溢出")?));
                    }
                    out.emit(
                        route_index,
                        output_stream,
                        copy,
                        base,
                        true,
                        progress,
                        cancelled,
                    )?;
                }
                _ => return Err("文件路由包含非文件编码阶段".into()),
            }
        }
        self.headers()?;
        Ok(true)
    }
    fn finish_route(&mut self, index: usize) -> Result<(), String> {
        if self.routes[index].finished {
            return Ok(());
        }
        let output = self.routes[index].output;
        let stream = self.routes[index].stream;
        let out = &mut self.outputs[output];
        let mut emit = |p, b| {
            out.emit(
                index,
                stream,
                p,
                b,
                false,
                &mut self.progress,
                &self.cancelled,
            )
        };
        match &mut self.routes[index].mode {
            Mode::FileVideo(s) => s.finish(&mut emit)?,
            Mode::FileAudio(s) => s.finish(&mut emit)?,
            Mode::Video(s) => s.finish(&mut emit)?,
            Mode::Audio(s) => s.finish(&mut emit)?,
            Mode::Copy(s) => s.done = true,
        }
        self.routes[index].finished = true;
        Ok(())
    }
    fn initialize_graph(&mut self) -> Result<(), String> {
        while let Some(decoder) = self
            .decoders
            .iter()
            .position(|d| d.template.is_none() && !d.eof)
        {
            if !self.decoders[decoder].advance(&self.cancelled)? {
                let input = self.decoders[decoder].input;
                self.read_input(input)?;
            }
        }
        for decoder in &mut self.decoders {
            if decoder.template.is_none() {
                decoder.template = decoder.empty_template()?;
            }
        }
        for binding in &mut self.bindings {
            if let Some(index) = binding.decoder {
                binding.template = Some(
                    self.decoders[index]
                        .template
                        .as_ref()
                        .ok_or("滤镜输入没有可解码帧")?
                        .reference()?,
                );
            }
        }
        self.configure_graph()?;
        self.headers()
    }
    fn pull_graph(&mut self) -> Result<bool, String> {
        let mut progress = false;
        let graph = self.graph.as_mut().ok_or("复杂滤镜图尚未配置")?;
        for sink_index in 0..graph.sinks.len() {
            if graph.finished[sink_index] {
                continue;
            }
            let (route_index, name) = &graph.sinks[sink_index];
            let route_index = *route_index;
            loop {
                util::check(&self.cancelled)?;
                let mut sink = graph.graph.get(name).ok_or("共享滤镜输出不存在")?;
                let mut frame = match self.routes[route_index].mode {
                    Mode::Video(_) => DecodedFrame::Video(frame::Video::empty()),
                    Mode::Audio(_) => DecodedFrame::Audio(frame::Audio::empty()),
                    _ => return Err("共享图输出路由类型不匹配".into()),
                };
                let result = perf::measure("cpu_filter", || unsafe {
                    ffi::av_buffersink_get_frame_flags(
                        sink.as_mut_ptr(),
                        frame.as_mut_ptr(),
                        ffi::AV_BUFFERSINK_FLAG_NO_REQUEST,
                    )
                });
                if result < 0 {
                    let error = ffmpeg::Error::from(result);
                    if error == ffmpeg::Error::Eof {
                        graph.finished[sink_index] = true;
                        break;
                    }
                    if util::again(error) {
                        break;
                    }
                    return Err(util::error("读取共享图输出失败", error));
                }
                progress = true;
                let base = sink.sink().time_base();
                let out_index = self.routes[route_index].output;
                let out_stream = self.routes[route_index].stream;
                let out = &mut self.outputs[out_index];
                let mut emit = |p, b| {
                    out.emit(
                        route_index,
                        out_stream,
                        p,
                        b,
                        false,
                        &mut self.progress,
                        &self.cancelled,
                    )
                };
                match (&mut self.routes[route_index].mode, frame) {
                    (Mode::Video(enc), DecodedFrame::Video(f)) => {
                        let rate = unsafe { ffi::av_buffersink_get_frame_rate(sink.as_mut_ptr()) };
                        let rate = if rate.num > 0 && rate.den > 0 {
                            rate.into()
                        } else {
                            Rational(0, 1)
                        };
                        enc.open_from_sink(&mut sink, &f, rate, base, self.device.as_ref())?;
                        enc.submit_filtered(f, base, &mut emit)?;
                    }
                    (Mode::Audio(enc), DecodedFrame::Audio(f)) => {
                        enc.submit_filtered(f, base, &mut emit)?
                    }
                    _ => return Err("共享图输出帧类型不匹配".into()),
                }
            }
        }
        let finished = self
            .graph
            .as_ref()
            .unwrap()
            .finished
            .iter()
            .enumerate()
            .filter(|(_, done)| **done)
            .map(|(i, _)| self.graph.as_ref().unwrap().sinks[i].0)
            .collect::<Vec<_>>();
        for index in finished {
            self.finish_route(index)?;
        }
        self.headers()?;
        Ok(progress)
    }
    fn feed_binding(&mut self, index: usize) -> Result<bool, String> {
        if self.bindings[index].flushed {
            return Ok(false);
        }
        let frame = if let Some(decoder) = self.bindings[index].decoder {
            if let Some(frame) = self.decoders[decoder].pop() {
                Some(frame)
            } else if self.decoders[decoder].eof {
                self.bindings[index].eof = true;
                None
            } else {
                if self.decoders[decoder].advance(&self.cancelled)? {
                    return Ok(true);
                }
                let input = self.decoders[decoder].input;
                return self.read_input(input);
            }
        } else if let Some(raw) = self.bindings[index].raw.as_mut() {
            match raw.next(&self.cancelled)? {
                Some(frame) => Some(DecodedFrame::Video(frame)),
                None => {
                    self.bindings[index].eof = true;
                    None
                }
            }
        } else {
            return Ok(false);
        };
        if let Some(mut frame) = frame {
            let changed = match &self.bindings[index].template {
                Some(old) => !old.matches(&frame)?,
                None => true,
            };
            if changed {
                self.bindings[index].template = Some(frame.reference()?);
                self.configure_graph()?;
            }
            // Display matrices have already been consumed by the input rotation
            // filters. Keeping them on submitted frames would rotate output twice.
            if let DecodedFrame::Video(f) = &mut frame {
                f.remove_side_data(frame::side_data::Type::DisplayMatrix);
            }
            let graph = self.graph.as_mut().unwrap();
            let mut source = graph
                .graph
                .get(&graph.sources[index])
                .ok_or("共享图输入不存在")?;
            let result = perf::measure("cpu_filter", || unsafe {
                ffi::av_buffersrc_add_frame_flags(
                    source.as_mut_ptr(),
                    frame.as_mut_ptr(),
                    ffi::AV_BUFFERSRC_FLAG_KEEP_REF as i32,
                )
            });
            if result < 0 && result != ffi::AVERROR_EOF {
                return Err(util::error(
                    "提交共享图输入失败",
                    ffmpeg::Error::from(result),
                ));
            }
            return Ok(true);
        }
        if self.bindings[index].eof {
            let graph = self.graph.as_mut().unwrap();
            let mut source = graph
                .graph
                .get(&graph.sources[index])
                .ok_or("共享图输入不存在")?;
            source
                .source()
                .flush()
                .map_err(|e| util::error("结束共享图输入失败", e))?;
            self.bindings[index].flushed = true;
            return Ok(true);
        }
        Ok(false)
    }
    fn run(mut self) -> Result<Completed, String> {
        perf::measure("initialize_graph", || {
            self.headers()?;
            if self.plan.filter_graph.is_some() {
                self.initialize_graph()?;
            }
            Ok::<_, String>(())
        })?;
        let mut round_robin = 0usize;
        loop {
            util::check(&self.cancelled)?;
            if self.routes.iter().all(Route::done) {
                break;
            }
            let mut did_work = false;
            if self.graph.is_some() {
                self.pull_graph()?;
                if !self.graph.as_ref().unwrap().finished.iter().all(|v| *v) {
                    let status = perf::measure("cpu_filter", || unsafe {
                        ffi::avfilter_graph_request_oldest(
                            self.graph.as_mut().unwrap().graph.as_mut_ptr(),
                        )
                    });
                    if status >= 0 {
                        continue;
                    }
                    if status != ffi::AVERROR_EOF && !util::again(ffmpeg::Error::from(status)) {
                        return Err(util::error(
                            "请求共享图输出失败",
                            ffmpeg::Error::from(status),
                        ));
                    }
                    self.pull_graph()?;
                    if !self.graph.as_ref().unwrap().finished.iter().all(|v| *v) {
                        let graph = self.graph.as_mut().unwrap();
                        let mut requests = Vec::new();
                        for (index, binding) in self.bindings.iter().enumerate() {
                            if binding.flushed
                                || matches!(
                                    self.plan.inputs[binding.input].source,
                                    MediaSource::GeneratedVideo(_) | MediaSource::GeneratedAudio(_)
                                )
                            {
                                continue;
                            }
                            let mut source = graph
                                .graph
                                .get(&graph.sources[index])
                                .ok_or("共享图输入不存在")?;
                            let failed = unsafe {
                                ffi::av_buffersrc_get_nb_failed_requests(source.as_mut_ptr())
                            };
                            requests.push((failed, index));
                        }
                        requests.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
                        for (_, index) in requests {
                            if self.feed_binding(index)? {
                                did_work = true;
                                break;
                            }
                        }
                    }
                }
            }
            if !did_work {
                let count = self.files.len();
                for step in 0..count {
                    let input = (round_robin + step) % count;
                    if self.files[input].as_ref().is_some_and(|f| !f.eof)
                        && self
                            .routes
                            .iter()
                            .any(|r| r.source.is_some_and(|(i, _)| i == input) && !r.done())
                    {
                        did_work = self.read_input(input)?;
                        round_robin = (input + 1) % count;
                        break;
                    }
                }
            }
            if !did_work {
                if self
                    .graph
                    .as_ref()
                    .is_some_and(|g| g.finished.iter().any(|v| !*v))
                {
                    return Err("共享滤镜图要求输入，但全部可读源已经结束".into());
                }
                break;
            }
        }
        for index in 0..self.routes.len() {
            self.finish_route(index)?;
        }
        self.headers()?;
        self.progress.emit(MediaStage::Finalizing, true);
        let finalizing = perf::span("mux_finalize");
        for output in &mut self.outputs {
            util::check(&self.cancelled)?;
            if !output.header {
                return Err("媒体流没有提供可写出的编码参数".into());
            }
            output
                .native
                .output
                .write_trailer()
                .map_err(|e| util::error("写入媒体文件尾失败", e))?;
            output.native.close()?;
        }
        drop(finalizing);
        util::check(&self.cancelled)?;
        let mut video_frames = 0;
        let mut audio_samples = 0;
        for route in &self.routes {
            match &route.mode {
                Mode::Video(s) => video_frames += s.frames(),
                Mode::FileVideo(s) => video_frames += s.frames(),
                Mode::Audio(s) => audio_samples += s.samples(),
                Mode::FileAudio(s) => audio_samples += s.samples(),
                _ => (),
            }
        }
        let files = self
            .outputs
            .iter_mut()
            .map(|o| (o.temp.take().unwrap(), o.target.clone()))
            .collect();
        let result = MediaJobResult {
            outputs: self.plan.outputs.iter().map(|o| o.path.clone()).collect(),
            video_frames,
            audio_samples,
            processed: self.progress.processed,
        };
        Ok(Completed {
            files,
            result,
            progress: self.progress,
            _resources: std::mem::take(&mut self.plan.resources),
        })
    }
}
