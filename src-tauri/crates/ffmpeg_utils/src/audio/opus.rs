//! The libopus profiles used for audio previews and service uploads. Filter
//! negotiation retains floating-point sources, and timestamps reach the Ogg
//! muxer unchanged so source delays and gaps remain on the playback timeline.

use std::{
    collections::VecDeque,
    ffi::{c_char, CStr, CString},
    ops::ControlFlow,
    path::Path,
    ptr,
    sync::{atomic::AtomicBool, Arc},
};

use ffmpeg_next::{self as ffmpeg, codec, ffi, filter, format, frame, Dictionary, Packet};
use tempfile::NamedTempFile;

use super::{
    again, audio_stream, check_cancelled, decode_frames, native_error, range::AudioClock,
    AudioFormat,
};
use crate::{
    metadata::{open_input, run_media},
    path_str,
};

#[derive(Clone, Copy)]
enum OpusProfile {
    Sample,
    Upload,
}

impl OpusProfile {
    fn bit_rate(self) -> usize {
        match self {
            Self::Sample => 64_000,
            Self::Upload => 128_000,
        }
    }
}

/// Encode the CLI's automatically selected audio stream as a 64 kbit/s VBR
/// Opus preview. Publish only after the worker has drained and closed its I/O.
pub async fn encode_opus_sample(input: &Path, output: &Path) -> Result<(), String> {
    encode_opus(input, output, OpusProfile::Sample).await
}

/// Encode a 128 kbit/s VBR Opus upload, negotiating the selected source's
/// sample rate and channel layout exactly as the FFmpeg CLI does by default.
pub async fn encode_opus_upload(input: &Path, output: &Path) -> Result<(), String> {
    encode_opus(input, output, OpusProfile::Upload).await
}

async fn encode_opus(input: &Path, output: &Path, profile: OpusProfile) -> Result<(), String> {
    let input = input.to_path_buf();
    let output = output.to_path_buf();
    let publish_path = output.clone();
    let temporary =
        run_media(move |cancelled| encode_sync(&input, &output, profile, cancelled)).await?;
    temporary
        .persist(&publish_path)
        .map_err(|error| native_error("发布 Opus 音频失败", error.error))?;
    Ok(())
}

fn encode_sync(
    input_path: &Path,
    output_path: &Path,
    profile: OpusProfile,
    cancelled: Arc<AtomicBool>,
) -> Result<NamedTempFile, String> {
    if input_path == output_path
        || std::fs::canonicalize(input_path)
            .ok()
            .zip(std::fs::canonicalize(output_path).ok())
            .is_some_and(|(input, output)| input == output)
    {
        return Err("Opus 输出不能覆盖音频输入".into());
    }
    let mut input = open_input(input_path, cancelled.clone())?;
    let index = audio_stream(&input).ok_or("Opus 音频提取失败: 输入没有音轨")?;
    let origin = unsafe { (*input.as_ptr()).start_time };
    let stream = input.stream(index).ok_or("找不到音轨")?;
    let packet_base: ffi::AVRational = stream.time_base().into();
    if packet_base.num <= 0 || packet_base.den <= 0 {
        return Err("音频流时基无效".into());
    }
    let global: Dictionary = input
        .metadata()
        .iter()
        .filter(|(key, _)| {
            ![
                "creation_time",
                "company_name",
                "product_name",
                "product_version",
            ]
            .iter()
            .any(|removed| key.eq_ignore_ascii_case(removed))
        })
        .collect();
    let metadata: Dictionary = stream.metadata().iter().collect();
    let context = codec::context::Context::from_parameters(stream.parameters())
        .map_err(|error| native_error("初始化音频解码器失败", error))?;
    let mut decoder = context.decoder();
    decoder.set_packet_time_base(stream.time_base());
    let mut decoder = decoder
        .audio()
        .map_err(|error| native_error("打开音频解码器失败", error))?;
    // Empty audio streams still negotiate an encoder and write headers, but
    // must not invent a silent frame. Keep only the decoder's format hints.
    let mut empty_frame = frame::Audio::empty();
    empty_frame.set_format(if decoder.format() == format::Sample::None {
        format::Sample::I16(format::sample::Type::Packed)
    } else {
        decoder.format()
    });
    empty_frame.set_rate(decoder.rate());
    unsafe {
        let result = ffi::av_channel_layout_copy(
            &mut (*empty_frame.as_mut_ptr()).ch_layout,
            &(*decoder.as_ptr()).ch_layout,
        );
        if result < 0 {
            return Err(native_error(
                "复制音频声道布局失败",
                ffmpeg::Error::from(result),
            ));
        }
    }
    let parent = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = tempfile::Builder::new()
        .prefix(".opus-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|error| native_error("创建 Opus 临时输出失败", error))?;
    // The guard is declared before all native contexts, so unwinding closes
    // their file handles before deleting the file, including on Windows.
    let mut output = OpusOutput::new(
        temporary.path(),
        profile,
        global,
        metadata,
        cancelled.clone(),
    )?;
    for chapter in input.chapters() {
        let offset = if origin == ffi::AV_NOPTS_VALUE {
            0
        } else {
            unsafe {
                ffi::av_rescale_q(
                    origin,
                    ffi::AVRational {
                        num: 1,
                        den: ffi::AV_TIME_BASE,
                    },
                    chapter.time_base().into(),
                )
            }
        };
        if chapter.end() < offset {
            continue;
        }
        let start = chapter
            .start()
            .checked_sub(offset)
            .ok_or("音频章节时间戳溢出")?
            .max(0);
        let end = chapter
            .end()
            .checked_sub(offset)
            .ok_or("音频章节时间戳溢出")?;
        let mut copied = output
            .muxer
            .output
            .add_chapter(chapter.id(), chapter.time_base(), start, end, "")
            .map_err(|error| native_error("复制音频章节失败", error))?;
        // add_chapter's mandatory title must not invent a tag for an untitled
        // source chapter. Copy its complete dictionary instead.
        unsafe {
            ffi::av_dict_free(&mut (*copied.as_mut_ptr()).metadata);
            let result = ffi::av_dict_copy(
                &mut (*copied.as_mut_ptr()).metadata,
                chapter.metadata().as_ptr(),
                0,
            );
            if result < 0 {
                return Err(native_error(
                    "复制音频章节标签失败",
                    ffmpeg::Error::from(result),
                ));
            }
        }
    }
    let offset = if origin == ffi::AV_NOPTS_VALUE {
        0
    } else {
        unsafe {
            ffi::av_rescale_q(
                origin.checked_neg().ok_or("音频时间戳溢出")?,
                ffi::AVRational {
                    num: 1,
                    den: ffi::AV_TIME_BASE,
                },
                packet_base,
            )
        }
    };
    decode_frames(
        &mut input,
        index,
        &mut decoder,
        &cancelled,
        |packet| {
            if let Some(pts) = packet.pts() {
                packet.set_pts(Some(pts.checked_add(offset).ok_or("音频时间戳溢出")?));
            }
            if let Some(dts) = packet.dts() {
                packet.set_dts(Some(dts.checked_add(offset).ok_or("音频时间戳溢出")?));
            }
            Ok(())
        },
        |frame| {
            output.frame(frame, packet_base)?;
            Ok(ControlFlow::Continue(()))
        },
    )?;
    output.finish(&empty_frame)?;
    drop(output);
    check_cancelled(&cancelled)?;
    Ok(temporary)
}

struct Muxer {
    output: format::context::Output,
    // AVIO keeps the opaque callback until output has closed.
    _interrupt: ffmpeg::util::interrupt::InterruptGuard,
}

impl Muxer {
    fn new(path: &Path, cancelled: Arc<AtomicBool>) -> Result<Self, String> {
        let filename = CString::new(path_str(path)?).map_err(|error| error.to_string())?;
        let token = cancelled;
        let interrupt = ffmpeg::util::interrupt::new(Box::new(move || {
            token.load(std::sync::atomic::Ordering::Acquire)
        }));
        let mut context = ptr::null_mut();
        let result = unsafe {
            ffi::avformat_alloc_output_context2(
                &mut context,
                ptr::null(),
                c"opus".as_ptr(),
                filename.as_ptr(),
            )
        };
        if result < 0 || context.is_null() {
            return Err(native_error(
                "创建 Ogg 输出失败",
                ffmpeg::Error::from(result),
            ));
        }
        // Wrap before opening AVIO, so a failed open still frees the format.
        let output = unsafe { format::context::Output::wrap(context) };
        unsafe {
            (*context).interrupt_callback = interrupt.interrupt;
            let result = ffi::avio_open2(
                &mut (*context).pb,
                filename.as_ptr(),
                ffi::AVIO_FLAG_WRITE,
                &(*context).interrupt_callback,
                ptr::null_mut(),
            );
            if result < 0 {
                return Err(native_error(
                    "打开 Opus 输出失败",
                    ffmpeg::Error::from(result),
                ));
            }
        }
        Ok(Self {
            output,
            _interrupt: interrupt.guard,
        })
    }

    fn close(&mut self) -> Result<(), String> {
        unsafe {
            let context = self.output.as_mut_ptr();
            ffi::avio_flush((*context).pb);
            let error = (*(*context).pb).error;
            let result = ffi::avio_closep(&mut (*context).pb);
            if error < 0 || result < 0 {
                return Err(native_error(
                    "关闭 Opus 输出失败",
                    ffmpeg::Error::from(if error < 0 { error } else { result }),
                ));
            }
        }
        Ok(())
    }
}

struct OpusOutput {
    muxer: Muxer,
    profile: OpusProfile,
    encoder: Option<codec::encoder::audio::Encoder>,
    encoder_format: Option<AudioFormat>,
    graph: Option<filter::Graph>,
    format: Option<AudioFormat>,
    pending: VecDeque<QueuedAudio>,
    pending_samples: usize,
    metadata: Option<Dictionary<'static>>,
    clock: AudioClock,
    cancelled: Arc<AtomicBool>,
}

struct QueuedAudio {
    frame: frame::Audio,
    offset: usize,
}

impl OpusOutput {
    fn new(
        path: &Path,
        profile: OpusProfile,
        global: Dictionary<'static>,
        metadata: Dictionary<'static>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let mut muxer = Muxer::new(path, cancelled.clone())?;
        muxer.output.set_metadata(global);
        Ok(Self {
            muxer,
            profile,
            encoder: None,
            encoder_format: None,
            graph: None,
            format: None,
            pending: VecDeque::new(),
            pending_samples: 0,
            metadata: Some(metadata),
            clock: AudioClock::new(),
            cancelled,
        })
    }

    fn open_encoder(&mut self, format: AudioFormat) -> Result<(), String> {
        let codec = ffmpeg::encoder::find_by_name("libopus").ok_or("FFmpeg 缺少 libopus 编码器")?;
        let mut encoder = codec::context::Context::new_with_codec(codec)
            .encoder()
            .audio()
            .map_err(|error| native_error("创建 Opus 编码器失败", error))?;
        encoder.set_rate(format.sample_rate);
        encoder.set_format(format.sample_format.into());
        // Set modern channel metadata directly; this works with FFmpeg 6.1
        // and 9 without depending on the wrapper's legacy channel fields.
        unsafe {
            let result =
                ffi::av_channel_layout_copy(&mut (*encoder.as_mut_ptr()).ch_layout, &format.layout);
            if result < 0 {
                return Err(native_error(
                    "复制 Opus 编码布局失败",
                    ffmpeg::Error::from(result),
                ));
            }
        }
        encoder.set_bit_rate(self.profile.bit_rate());
        encoder.set_time_base((1, format.sample_rate));
        if self
            .muxer
            .output
            .format()
            .flags()
            .contains(format::flag::Flags::GLOBAL_HEADER)
        {
            encoder.set_flags(codec::flag::Flags::GLOBAL_HEADER);
        }
        let mut options = Dictionary::new();
        options.set("vbr", "on");
        options.set("compression_level", "10");
        let encoder = encoder
            .open_as_with(codec, options)
            .map_err(|error| native_error("打开 libopus 编码器失败", error))?;
        let mut stream = self
            .muxer
            .output
            .add_stream(codec)
            .map_err(|error| native_error("创建 Opus 音轨失败", error))?;
        stream.set_parameters(&encoder);
        stream.set_time_base((1, format.sample_rate));
        let mut metadata = self.metadata.take().unwrap_or_default();
        let version = ffmpeg::codec::version();
        metadata.set(
            "encoder",
            &format!(
                "Lavc{}.{}.{} libopus",
                version >> 16,
                (version >> 8) & 255,
                version & 255
            ),
        );
        stream.set_metadata(metadata);
        self.muxer
            .output
            .write_header()
            .map_err(|error| native_error("写入 Opus 文件头失败", error))?;
        self.encoder = Some(encoder);
        self.encoder_format = Some(format);
        Ok(())
    }

    fn frame(
        &mut self,
        frame: &mut frame::Audio,
        packet_base: ffi::AVRational,
    ) -> Result<(), String> {
        check_cancelled(&self.cancelled)?;
        if frame.samples() == 0 {
            return Ok(());
        }
        let format = AudioFormat::from_frame(frame)?;
        self.clock.normalize(frame, packet_base)?;
        if self
            .format
            .as_ref()
            .is_some_and(|previous| !previous.matches(&format))
        {
            // ffmpeg_filter.c reinitializes after draining ready frames,
            // without flushing the previous resampler's delayed samples.
            self.graph = None;
            self.format = None;
        }
        if self.graph.is_none() {
            self.configure(frame, format)?;
        }
        unsafe {
            if (*frame.as_ptr()).ch_layout.order == ffi::AVChannelOrder::AV_CHANNEL_ORDER_UNSPEC {
                ffi::av_channel_layout_uninit(&mut (*frame.as_mut_ptr()).ch_layout);
                let result = ffi::av_channel_layout_copy(
                    &mut (*frame.as_mut_ptr()).ch_layout,
                    &self.format.as_ref().unwrap().layout,
                );
                if result < 0 {
                    return Err(native_error(
                        "复制音频声道失败",
                        ffmpeg::Error::from(result),
                    ));
                }
            }
        }
        let mut source = self
            .graph
            .as_mut()
            .unwrap()
            .get("in")
            .ok_or("Opus 滤镜输入不存在")?;
        let result = unsafe {
            ffi::av_buffersrc_add_frame_flags(
                source.as_mut_ptr(),
                frame.as_mut_ptr(),
                ffi::AV_BUFFERSRC_FLAG_KEEP_REF as i32,
            )
        };
        if result < 0 {
            return Err(native_error(
                "提交 Opus 滤镜失败",
                ffmpeg::Error::from(result),
            ));
        }
        self.pull()
    }

    fn configure(&mut self, frame: &frame::Audio, format: AudioFormat) -> Result<(), String> {
        let mut graph = make_graph(frame, &format, self.profile, self.encoder_format.as_ref())?;
        if self.encoder.is_none() {
            let mut output = graph.get("out").ok_or("Opus 滤镜输出不存在")?;
            let mut negotiated = AudioFormat {
                layout: unsafe { std::mem::zeroed() },
                sample_format: ffi::AVSampleFormat::AV_SAMPLE_FMT_NONE,
                sample_rate: unsafe { ffi::av_buffersink_get_sample_rate(output.as_mut_ptr()) },
            };
            let sample_format = unsafe { ffi::av_buffersink_get_format(output.as_mut_ptr()) };
            negotiated.sample_format = match sample_format {
                value if value == ffi::AVSampleFormat::AV_SAMPLE_FMT_S16 as i32 => {
                    ffi::AVSampleFormat::AV_SAMPLE_FMT_S16
                }
                value if value == ffi::AVSampleFormat::AV_SAMPLE_FMT_FLT as i32 => {
                    ffi::AVSampleFormat::AV_SAMPLE_FMT_FLT
                }
                _ => return Err("Opus 滤镜协商了不支持的采样格式".into()),
            };
            let result = unsafe {
                ffi::av_buffersink_get_ch_layout(output.as_mut_ptr(), &mut negotiated.layout)
            };
            if result < 0 {
                return Err(native_error(
                    "读取 Opus 协商声道失败",
                    ffmpeg::Error::from(result),
                ));
            }
            if negotiated.sample_rate <= 0 || negotiated.layout.nb_channels <= 0 {
                return Err("Opus 滤镜协商的音频格式无效".into());
            }
            self.open_encoder(negotiated)?;
        }
        self.graph = Some(graph);
        self.format = Some(format);
        Ok(())
    }

    fn pull(&mut self) -> Result<(), String> {
        let mut frame = frame::Audio::empty();
        loop {
            check_cancelled(&self.cancelled)?;
            let mut output = self
                .graph
                .as_mut()
                .unwrap()
                .get("out")
                .ok_or("Opus 滤镜输出不存在")?;
            match output.sink().frame(&mut frame) {
                Ok(()) => (),
                Err(ffmpeg::Error::Eof) => break,
                Err(error) if again(error) => break,
                Err(error) => return Err(native_error("读取 Opus 滤镜失败", error)),
            }
            if frame.samples() == 0 {
                continue;
            }
            let base: ffi::AVRational = output.sink().time_base().into();
            let pts = frame.pts().ok_or("Opus 滤镜未返回时间戳")?;
            let sample_base = ffi::AVRational {
                num: 1,
                den: self.encoder_format.as_ref().unwrap().sample_rate,
            };
            frame.set_pts(Some(unsafe { ffi::av_rescale_q(pts, base, sample_base) }));
            unsafe {
                (*frame.as_mut_ptr()).time_base = sample_base;
                (*frame.as_mut_ptr()).duration = frame.samples() as i64;
            }
            self.pending_samples = self
                .pending_samples
                .checked_add(frame.samples())
                .ok_or("Opus 音频队列溢出")?;
            self.pending.push_back(QueuedAudio { frame, offset: 0 });
            self.reblock(false)?;
            frame = frame::Audio::empty();
        }
        Ok(())
    }

    fn reblock(&mut self, eof: bool) -> Result<(), String> {
        let encoder = self.encoder.as_ref().ok_or("Opus 编码器不存在")?;
        let frame_size = encoder.frame_size() as usize;
        let sample_format = encoder.format();
        let sample_rate = encoder.rate() as i32;
        let channels = self.encoder_format.as_ref().unwrap().layout.nb_channels as usize;
        if frame_size == 0 {
            return Err("Opus 编码器帧长度无效".into());
        }
        while self.pending_samples >= frame_size || (eof && self.pending_samples != 0) {
            check_cancelled(&self.cancelled)?;
            let count = self.pending_samples.min(frame_size);
            let first = self.pending.front().ok_or("Opus 音频队列为空")?;
            let mut combined = frame::Audio::empty();
            // Match fftools/sync_queue.c receive_samples: copy complete
            // source blocks without filling timestamp gaps. A combined frame
            // inherits the first remaining block's PTS and properties, and
            // partial consumption advances that block on the sample grid.
            unsafe {
                let result = ffi::av_frame_copy_props(combined.as_mut_ptr(), first.frame.as_ptr());
                if result < 0 {
                    return Err(native_error(
                        "复制 Opus 帧属性失败",
                        ffmpeg::Error::from(result),
                    ));
                }
                let raw = &mut *combined.as_mut_ptr();
                raw.format = ffi::AVSampleFormat::from(sample_format) as i32;
                raw.sample_rate = sample_rate;
                raw.nb_samples = count as i32;
                raw.time_base = ffi::AVRational {
                    num: 1,
                    den: sample_rate,
                };
                raw.pts = first
                    .frame
                    .pts()
                    .ok_or("Opus 音频队列缺少时间戳")?
                    .checked_add(first.offset as i64)
                    .ok_or("音频时间戳溢出")?;
                raw.duration = count as i64;
                let result = ffi::av_channel_layout_copy(
                    &mut raw.ch_layout,
                    &(*first.frame.as_ptr()).ch_layout,
                );
                if result < 0 {
                    return Err(native_error(
                        "复制 Opus 帧声道失败",
                        ffmpeg::Error::from(result),
                    ));
                }
                let result = ffi::av_frame_get_buffer(combined.as_mut_ptr(), 0);
                if result < 0 {
                    return Err(native_error(
                        "分配 Opus 音频帧失败",
                        ffmpeg::Error::from(result),
                    ));
                }
            }
            // libopus accepts packed s16/float. Each sample frame contains
            // all negotiated channels, including supported surround layouts.
            let bytes = sample_format
                .bytes()
                .checked_mul(channels)
                .ok_or("Opus 样本长度溢出")?;
            let mut copied = 0;
            while copied < count {
                let first = self.pending.front_mut().ok_or("Opus 音频队列为空")?;
                let available = first.frame.samples() - first.offset;
                let take = available.min(count - copied);
                unsafe {
                    ptr::copy_nonoverlapping(
                        (*first.frame.as_ptr()).data[0].add(first.offset * bytes),
                        (*combined.as_mut_ptr()).data[0].add(copied * bytes),
                        take * bytes,
                    );
                }
                copied += take;
                first.offset += take;
                if first.offset == first.frame.samples() {
                    self.pending.pop_front();
                }
            }
            self.pending_samples -= count;
            match self.encoder.as_mut().unwrap().send_frame(&combined) {
                Ok(()) => (),
                Err(error) if again(error) => {
                    self.packets()?;
                    self.encoder
                        .as_mut()
                        .unwrap()
                        .send_frame(&combined)
                        .map_err(|error| native_error("提交 Opus 编码帧失败", error))?;
                }
                Err(error) => return Err(native_error("提交 Opus 编码帧失败", error)),
            }
            self.packets()?;
        }
        Ok(())
    }

    fn packets(&mut self) -> Result<bool, String> {
        loop {
            check_cancelled(&self.cancelled)?;
            let mut packet = Packet::empty();
            match self.encoder.as_mut().unwrap().receive_packet(&mut packet) {
                Ok(()) => (),
                Err(ffmpeg::Error::Eof) => return Ok(true),
                Err(error) if again(error) => return Ok(false),
                Err(error) => return Err(native_error("读取 Opus 编码包失败", error)),
            }
            let output_base = self
                .muxer
                .output
                .stream(0)
                .ok_or("Opus 音轨不存在")?
                .time_base();
            // Keep duration and AV_PKT_DATA_SKIP_SAMPLES from libopus: the
            // Ogg muxer needs them for pre-skip and the final granule position.
            packet.rescale_ts(self.encoder.as_ref().unwrap().time_base(), output_base);
            packet.set_stream(0);
            packet
                .write_interleaved(&mut self.muxer.output)
                .map_err(|error| native_error("写入 Opus 编码包失败", error))?;
        }
    }

    fn finish(&mut self, empty_frame: &frame::Audio) -> Result<(), String> {
        if let Some(graph) = self.graph.as_mut() {
            graph
                .get("in")
                .ok_or("Opus 滤镜输入不存在")?
                .source()
                .flush()
                .map_err(|error| native_error("结束 Opus 滤镜失败", error))?;
            self.pull()?;
        }
        if self.encoder.is_none() {
            self.configure(empty_frame, AudioFormat::from_frame(empty_frame)?)?;
        }
        self.reblock(true)?;
        match self.encoder.as_mut().unwrap().send_eof() {
            Ok(()) | Err(ffmpeg::Error::Eof) => (),
            Err(error) if again(error) => {
                self.packets()?;
                self.encoder
                    .as_mut()
                    .unwrap()
                    .send_eof()
                    .map_err(|error| native_error("结束 Opus 编码失败", error))?;
            }
            Err(error) => return Err(native_error("结束 Opus 编码失败", error)),
        }
        if !self.packets()? {
            return Err("Opus 编码器结束后仍要求输入".into());
        }
        check_cancelled(&self.cancelled)?;
        self.muxer
            .output
            .write_trailer()
            .map_err(|error| native_error("写入 Opus 文件尾失败", error))?;
        self.muxer.close()
    }
}

fn make_graph(
    frame: &frame::Audio,
    format: &AudioFormat,
    profile: OpusProfile,
    locked: Option<&AudioFormat>,
) -> Result<filter::Graph, String> {
    let layout = describe_layout(&format.layout)?;
    let mut graph = filter::Graph::new();
    unsafe {
        (*graph.as_mut_ptr()).nb_threads = 1;
    }
    graph
        .add(
            &filter::find("abuffer").ok_or("FFmpeg 缺少 abuffer 滤镜")?,
            "in",
            &format!(
                "time_base=1/{0}:sample_rate={0}:sample_fmt={1}:channel_layout={layout}",
                frame.rate(),
                frame.format().name()
            ),
        )
        .map_err(|error| native_error("创建 Opus 滤镜输入失败", error))?;
    graph
        .add(
            &filter::find("abuffersink").ok_or("FFmpeg 缺少 abuffersink 滤镜")?,
            "out",
            "",
        )
        .map_err(|error| native_error("创建 Opus 滤镜输出失败", error))?;
    let spec = if let Some(locked) = locked {
        let format = format::Sample::from(locked.sample_format);
        let layout = describe_layout(&locked.layout)?;
        format!(
            "aformat=sample_fmts={}:sample_rates={}:channel_layouts={layout}",
            format.name(),
            locked.sample_rate
        )
    } else {
        match profile {
            OpusProfile::Sample => {
                "aformat=sample_fmts=s16|flt:sample_rates=16000:channel_layouts=mono".to_string()
            }
            OpusProfile::Upload => {
                let codec = ffmpeg::encoder::find_by_name("libopus")
                    .ok_or("FFmpeg 缺少 libopus 编码器")?
                    .audio()
                    .map_err(|error| native_error("读取 libopus 能力失败", error))?;
                let mut spec = "aformat=sample_fmts=s16|flt".to_string();
                if let Some(rates) = codec.rates() {
                    // Keep the codec's preference order. The filter graph's
                    // closest-rate negotiation also uses that order for ties.
                    let rates = rates
                        .map(|rate| rate.to_string())
                        .collect::<Vec<_>>()
                        .join("|");
                    if rates.is_empty() {
                        return Err("libopus 未提供可用采样率".into());
                    }
                    spec.push_str(":sample_rates=");
                    spec.push_str(&rates);
                }
                // Without -ac, the CLI preserves the source layout. libopus
                // validates its default mapping family rather than silently
                // downmixing unsupported layouts to mono or stereo.
                spec
            }
        }
    };
    graph
        .output("in", 0)
        .and_then(|parser| parser.input("out", 0))
        .and_then(|parser| parser.parse(&spec))
        .map_err(|error| native_error("创建 Opus 滤镜失败", error))?;
    graph
        .validate()
        .map_err(|error| native_error("配置 Opus 滤镜失败", error))?;
    Ok(graph)
}

fn describe_layout(layout: &ffi::AVChannelLayout) -> Result<String, String> {
    let mut description = vec![0 as c_char; 256];
    let length = unsafe {
        ffi::av_channel_layout_describe(layout, description.as_mut_ptr(), description.len())
    };
    if length < 0 {
        return Err("无法描述音频声道布局".into());
    }
    if length as usize >= description.len() {
        description.resize(length as usize + 1, 0);
        let result = unsafe {
            ffi::av_channel_layout_describe(layout, description.as_mut_ptr(), description.len())
        };
        if result < 0 {
            return Err("无法描述音频声道布局".into());
        }
    }
    let layout = unsafe { CStr::from_ptr(description.as_ptr()) }
        .to_str()
        .map_err(|error| error.to_string())?;
    Ok(layout.to_string())
}
