//! Sample-aligned FLAC previews. Keep the input clock during preroll, then
//! reproduce the editor's AAC-grid trim before resetting and resampling it.

use std::{
    ffi::{c_char, CStr, CString},
    ops::ControlFlow,
    path::Path,
    sync::{atomic::AtomicBool, Arc},
};

use ffmpeg_next::{self as ffmpeg, codec, ffi, filter, format, frame, media, Dictionary, Packet};
use tempfile::NamedTempFile;

use super::{
    again, check_cancelled, decode_frames, native_error, output::NativeOutput, range::AudioClock,
    AudioFormat, AudioRange,
};
use crate::metadata::{open_input, run_media};

const RATE: i32 = 48_000;
const SAMPLE_BASE: ffi::AVRational = ffi::AVRational { num: 1, den: RATE };
const AAC_FRAME_SAMPLES: i64 = 1024;

/// The first audio stream's sample clock, as used by the editor's preview
/// cache. `start` is the position of the first decoded sample on that clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioTiming {
    pub sample_rate: u32,
    pub start: i64,
    pub aac: bool,
}

/// Encode a sample-aligned preview as 48 kHz stereo s16 FLAC. Preroll keeps
/// codec history, while fixed 4096-sample blocks remain readable by WebKit.
pub async fn encode_flac_preview(
    input: &Path,
    output: &Path,
    timing: AudioTiming,
    range: AudioRange,
    pre_roll: f64,
) -> Result<(), String> {
    let filters = preview_filters(timing, range)?;
    if !pre_roll.is_finite() || pre_roll < 0.0 {
        return Err("无效音频前滚时间".into());
    }
    let seek = range.start - pre_roll;
    let seek = if seek > 0.0 {
        // The old command formats its input -ss to six decimal places before
        // parsing it. It deliberately never seeks when starting from zero.
        let argument = CString::new(format!("{seek:.6}")).map_err(|error| error.to_string())?;
        let mut micros = 0;
        let result = unsafe { ffi::av_parse_time(&mut micros, argument.as_ptr(), 1) };
        if result < 0 || micros < 0 {
            return Err("无效音频定位时间".into());
        }
        Some(micros)
    } else {
        None
    };
    let input = input.to_path_buf();
    let output = output.to_path_buf();
    let publish = output.clone();
    let temporary =
        run_media(move |cancelled| encode_sync(&input, &output, &filters, seek, cancelled)).await?;
    temporary
        .persist(publish)
        .map_err(|error| native_error("发布 FLAC 预览失败", error.error))?;
    Ok(())
}

fn preview_filters(timing: AudioTiming, range: AudioRange) -> Result<String, String> {
    let rate = timing.sample_rate;
    if rate == 0 || rate > i32::MAX as u32 {
        return Err("音频采样率无效".into());
    }
    let samples = |seconds: f64| -> Result<i64, String> {
        let samples = (seconds * f64::from(rate)).round();
        if !seconds.is_finite() || seconds < 0.0 || samples >= i64::MAX as f64 {
            return Err("无效音频预览时间范围".into());
        }
        Ok(samples as i64)
    };
    let from = samples(range.start)?;
    let to = from
        .checked_add(samples(range.duration)?)
        .ok_or("音频预览时间范围溢出")?;
    let origin = timing.start;
    let first = if timing.aac {
        format!("round((STARTPTS-({origin}))/{AAC_FRAME_SAMPLES})*{AAC_FRAME_SAMPLES}")
    } else {
        format!("round(STARTPTS)-({origin})")
    };
    // aformat is the CLI's output negotiation, after the user filter graph.
    // Keeping it after asetnsamples preserves resampling and block boundaries.
    Ok(format!("asettb=1/{rate},asetpts='{first}+N',atrim=start_pts={from}:end_pts={to},asetpts=PTS-STARTPTS,aresample={RATE},asetnsamples=n=4096:p=0,aformat=sample_fmts=s16:sample_rates={RATE}:channel_layouts=stereo"))
}

fn encode_sync(
    input_path: &Path,
    output_path: &Path,
    filters: &str,
    seek: Option<i64>,
    cancelled: Arc<AtomicBool>,
) -> Result<NamedTempFile, String> {
    if input_path == output_path
        || std::fs::canonicalize(input_path)
            .ok()
            .zip(std::fs::canonicalize(output_path).ok())
            .is_some_and(|(input, output)| input == output)
    {
        return Err("FLAC 预览输出不能覆盖输入".into());
    }
    let mut input = open_input(input_path, cancelled.clone())?;
    // The preview has always used -map 0:a:0, independently of dispositions
    // and channel counts used by automatic audio selection elsewhere.
    let index = input
        .streams()
        .find(|stream| stream.parameters().medium() == media::Type::Audio)
        .map(|stream| stream.index())
        .ok_or("FLAC 预览输入没有音轨")?;
    if let Some(seek) = seek {
        let origin = unsafe { (*input.as_ptr()).start_time };
        let target = seek
            .checked_add(if origin == ffi::AV_NOPTS_VALUE {
                0
            } else {
                origin
            })
            .ok_or("音频定位时间溢出")?;
        let dts_preroll = unsafe { (*input.format().as_ptr()).flags & ffi::AVFMT_SEEK_TO_PTS == 0 }
            && input
                .streams()
                .any(|stream| unsafe { (*stream.parameters().as_ptr()).video_delay != 0 });
        let target = if dts_preroll {
            target
                .checked_sub(3 * i64::from(ffi::AV_TIME_BASE) / 23)
                .ok_or("音频定位时间溢出")?
        } else {
            target
        };
        // ffmpeg_demux.c warns on unsupported/failed seeks and continues.
        let _seek_result = input.seek(target, ..target);
    }
    check_cancelled(&cancelled)?;
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
    let parent = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = tempfile::Builder::new()
        .prefix(".preview-flac-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|error| native_error("创建 FLAC 预览临时输出失败", error))?;
    let mut output = PreviewOutput::new(
        temporary.path(),
        filters,
        global,
        metadata,
        cancelled.clone(),
    )?;
    // With -copyts (and no -start_at_zero), the CLI leaves packet timestamps
    // untouched even after a seek. The first filter supplies the sample origin.
    decode_frames(
        &mut input,
        index,
        &mut decoder,
        &cancelled,
        |_| Ok(()),
        |frame| output.frame(frame, packet_base),
    )?;
    output.finish()?;
    drop(output);
    check_cancelled(&cancelled)?;
    Ok(temporary)
}

struct PreviewOutput {
    muxer: NativeOutput,
    encoder: Option<codec::encoder::audio::Encoder>,
    graph: Option<filter::Graph>,
    format: Option<AudioFormat>,
    metadata: Option<Dictionary<'static>>,
    filters: String,
    clock: AudioClock,
    complete: bool,
    cancelled: Arc<AtomicBool>,
}

impl PreviewOutput {
    fn new(
        path: &Path,
        filters: &str,
        global: Dictionary<'static>,
        metadata: Dictionary<'static>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let mut muxer = NativeOutput::new(path, "flac", cancelled.clone())?;
        muxer.output.set_metadata(global);
        Ok(Self {
            muxer,
            encoder: None,
            graph: None,
            format: None,
            metadata: Some(metadata),
            filters: filters.to_string(),
            clock: AudioClock::new(),
            complete: false,
            cancelled,
        })
    }

    fn open_encoder(&mut self, first_samples: usize) -> Result<(), String> {
        let codec = ffmpeg::encoder::find_by_name("flac").ok_or("FFmpeg 缺少 FLAC 编码器")?;
        let mut encoder = codec::context::Context::new_with_codec(codec)
            .encoder()
            .audio()
            .map_err(|error| native_error("创建 FLAC 编码器失败", error))?;
        encoder.set_format(format::Sample::I16(format::sample::Type::Packed));
        encoder.set_rate(RATE);
        encoder.set_time_base((1, RATE));
        // fftools/ffmpeg_enc.c takes the initial filtered frame's size before
        // opening a fixed-frame codec. Usually 4096; a short clip can be less.
        unsafe {
            (*encoder.as_mut_ptr()).frame_size = first_samples as i32;
            ffi::av_channel_layout_default(&mut (*encoder.as_mut_ptr()).ch_layout, 2);
        }
        if self
            .muxer
            .output
            .format()
            .flags()
            .contains(format::flag::Flags::GLOBAL_HEADER)
        {
            encoder.set_flags(codec::flag::Flags::GLOBAL_HEADER);
        }
        let encoder = encoder
            .open_as(codec)
            .map_err(|error| native_error("打开 FLAC 编码器失败", error))?;
        let mut stream = self
            .muxer
            .output
            .add_stream(codec)
            .map_err(|error| native_error("创建 FLAC 音轨失败", error))?;
        stream.set_parameters(&encoder);
        stream.set_time_base((1, RATE));
        let mut metadata = self.metadata.take().unwrap_or_default();
        let version = ffmpeg::codec::version();
        metadata.set(
            "encoder",
            &format!(
                "Lavc{}.{}.{} flac",
                version >> 16,
                (version >> 8) & 255,
                version & 255
            ),
        );
        stream.set_metadata(metadata);
        self.muxer
            .output
            .write_header()
            .map_err(|error| native_error("写入 FLAC 文件头失败", error))?;
        self.encoder = Some(encoder);
        Ok(())
    }

    fn frame(
        &mut self,
        frame: &mut frame::Audio,
        packet_base: ffi::AVRational,
    ) -> Result<ControlFlow<()>, String> {
        check_cancelled(&self.cancelled)?;
        if frame.samples() == 0 {
            return Ok(ControlFlow::Continue(()));
        }
        let format = AudioFormat::from_frame(frame)?;
        self.clock.normalize(frame, packet_base)?;
        if self
            .format
            .as_ref()
            .is_some_and(|previous| !previous.matches(&format))
        {
            // The CLI drops the old graph after pulling ready frames, without
            // flushing its delayed resampling samples on input reconfiguration.
            self.graph = None;
            self.format = None;
        }
        if self.graph.is_none() {
            self.graph = Some(make_graph(frame, &format, &self.filters)?);
            self.format = Some(format);
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
            .ok_or("FLAC 滤镜输入不存在")?;
        let result = unsafe {
            ffi::av_buffersrc_add_frame_flags(
                source.as_mut_ptr(),
                frame.as_mut_ptr(),
                ffi::AV_BUFFERSRC_FLAG_KEEP_REF as i32,
            )
        };
        if result == ffi::AVERROR_EOF {
            self.complete = true;
        } else if result < 0 {
            return Err(native_error(
                "提交 FLAC 滤镜失败",
                ffmpeg::Error::from(result),
            ));
        }
        self.pull()?;
        Ok(if self.complete {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        })
    }

    fn pull(&mut self) -> Result<(), String> {
        let mut frame = frame::Audio::empty();
        loop {
            check_cancelled(&self.cancelled)?;
            let mut sink = self
                .graph
                .as_mut()
                .unwrap()
                .get("out")
                .ok_or("FLAC 滤镜输出不存在")?;
            match sink.sink().frame(&mut frame) {
                Ok(()) => (),
                Err(ffmpeg::Error::Eof) => {
                    self.complete = true;
                    break;
                }
                Err(error) if again(error) => break,
                Err(error) => return Err(native_error("读取 FLAC 滤镜失败", error)),
            }
            if frame.samples() == 0 {
                continue;
            }
            let base: ffi::AVRational = sink.sink().time_base().into();
            let pts = frame.pts().ok_or("FLAC 滤镜未返回时间戳")?;
            frame.set_pts(Some(unsafe { ffi::av_rescale_q(pts, base, SAMPLE_BASE) }));
            unsafe {
                (*frame.as_mut_ptr()).time_base = SAMPLE_BASE;
                (*frame.as_mut_ptr()).duration = frame.samples() as i64;
            }
            if self.encoder.is_none() {
                self.open_encoder(frame.samples())?;
            }
            match self.encoder.as_mut().unwrap().send_frame(&frame) {
                Ok(()) => (),
                Err(error) if again(error) => {
                    self.packets()?;
                    self.encoder
                        .as_mut()
                        .unwrap()
                        .send_frame(&frame)
                        .map_err(|error| native_error("提交 FLAC 编码帧失败", error))?;
                }
                Err(error) => return Err(native_error("提交 FLAC 编码帧失败", error)),
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
                Err(error) => return Err(native_error("读取 FLAC 编码包失败", error)),
            }
            // The delayed FLAC encoder's final packet can contain new
            // STREAMINFO side data. Preserve it for the muxer's MD5/length fixup.
            packet.rescale_ts(
                (1, RATE),
                self.muxer
                    .output
                    .stream(0)
                    .ok_or("FLAC 音轨不存在")?
                    .time_base(),
            );
            packet.set_stream(0);
            packet
                .write_interleaved(&mut self.muxer.output)
                .map_err(|error| native_error("写入 FLAC 编码包失败", error))?;
        }
    }

    fn finish(&mut self) -> Result<(), String> {
        if !self.complete {
            if let Some(graph) = self.graph.as_mut() {
                graph
                    .get("in")
                    .ok_or("FLAC 滤镜输入不存在")?
                    .source()
                    .flush()
                    .map_err(|error| native_error("结束 FLAC 滤镜失败", error))?;
                self.pull()?;
            }
        }
        if self.encoder.is_none() {
            self.open_encoder(0)?;
        }
        match self.encoder.as_mut().unwrap().send_eof() {
            Ok(()) | Err(ffmpeg::Error::Eof) => (),
            Err(error) if again(error) => {
                self.packets()?;
                self.encoder
                    .as_mut()
                    .unwrap()
                    .send_eof()
                    .map_err(|error| native_error("结束 FLAC 编码失败", error))?;
            }
            Err(error) => return Err(native_error("结束 FLAC 编码失败", error)),
        }
        if !self.packets()? {
            return Err("FLAC 编码器结束后仍要求输入".into());
        }
        check_cancelled(&self.cancelled)?;
        self.muxer
            .output
            .write_trailer()
            .map_err(|error| native_error("写入 FLAC 文件尾失败", error))?;
        self.muxer.close()
    }
}

fn make_graph(
    frame: &frame::Audio,
    format: &AudioFormat,
    filters: &str,
) -> Result<filter::Graph, String> {
    let mut description = vec![0 as c_char; 256];
    let length = unsafe {
        ffi::av_channel_layout_describe(&format.layout, description.as_mut_ptr(), description.len())
    };
    if length < 0 {
        return Err("无法描述音频声道布局".into());
    }
    if length as usize >= description.len() {
        description.resize(length as usize + 1, 0);
        let result = unsafe {
            ffi::av_channel_layout_describe(
                &format.layout,
                description.as_mut_ptr(),
                description.len(),
            )
        };
        if result < 0 {
            return Err("无法描述音频声道布局".into());
        }
    }
    let layout = unsafe { CStr::from_ptr(description.as_ptr()) }
        .to_str()
        .map_err(|error| error.to_string())?;
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
        .map_err(|error| native_error("创建 FLAC 滤镜输入失败", error))?;
    graph
        .add(
            &filter::find("abuffersink").ok_or("FFmpeg 缺少 abuffersink 滤镜")?,
            "out",
            "",
        )
        .map_err(|error| native_error("创建 FLAC 滤镜输出失败", error))?;
    graph
        .output("in", 0)
        .and_then(|parser| parser.input("out", 0))
        .and_then(|parser| parser.parse(filters))
        .map_err(|error| native_error("创建 FLAC 预览滤镜失败", error))?;
    graph
        .validate()
        .map_err(|error| native_error("配置 FLAC 预览滤镜失败", error))?;
    Ok(graph)
}
