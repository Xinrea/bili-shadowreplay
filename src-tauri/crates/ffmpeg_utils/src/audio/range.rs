//! Native counterpart of the application's `-ss ... -i ... -t ...` WAV jobs.
//! FFmpeg's CLI combines two limits: an atrim duration relative to the first
//! accepted frame, and an encoder cutoff based on each output frame's PTS.

use std::{
    ffi::{c_char, CStr, CString},
    ops::ControlFlow,
    path::Path,
    sync::{atomic::AtomicBool, Arc},
};

use ffmpeg_next::{self as ffmpeg, codec, ffi, filter, frame, Packet};

use super::{
    again, audio_stream, check_cancelled, decode_frames, native_error, AudioDecodeOptions,
    AudioDecodeResult, AudioFormat, AudioSink,
};
use crate::metadata::{open_input, run_media};

#[derive(Debug, Clone, Copy)]
pub struct AudioRange {
    pub start: f64,
    pub duration: f64,
}

/// Decode an input-relative time range with the former FFmpeg CLI's accurate
/// seek and PCM output timing, including priming and whole output-frame tails.
pub async fn decode_audio_range<S: AudioSink>(
    path: &Path,
    options: AudioDecodeOptions,
    range: AudioRange,
    sink: S,
) -> Result<AudioDecodeResult<S>, String> {
    if !(1..=384_000).contains(&options.sample_rate) {
        return Err("音频采样率必须在 1 到 384000 之间".into());
    }
    let start = parse_seconds(range.start)?;
    let duration = parse_seconds(range.duration)?;
    let path = path.to_path_buf();
    run_media(move |cancelled| {
        decode_range_sync(&path, options.sample_rate, start, duration, sink, cancelled)
    })
    .await
}

fn parse_seconds(value: f64) -> Result<i64, String> {
    if !value.is_finite() || value < 0.0 {
        return Err("无效音频提取时间范围".into());
    }
    // Use exactly the parser formerly fed by the CLI's f64::to_string argv.
    let argument = CString::new(value.to_string()).map_err(|error| error.to_string())?;
    let mut micros = 0;
    let result = unsafe { ffi::av_parse_time(&mut micros, argument.as_ptr(), 1) };
    if result < 0 || micros < 0 {
        return Err("无效音频提取时间范围".into());
    }
    Ok(micros)
}

const MICROS: ffi::AVRational = ffi::AVRational {
    num: 1,
    den: ffi::AV_TIME_BASE,
};

fn decode_range_sync<S: AudioSink>(
    path: &Path,
    rate: u32,
    start: i64,
    duration: i64,
    mut sink: S,
    cancelled: Arc<AtomicBool>,
) -> Result<AudioDecodeResult<S>, String> {
    let mut input = open_input(path, cancelled.clone())?;
    let Some(index) = audio_stream(&input) else {
        check_cancelled(&cancelled)?;
        sink.finish()?;
        return Ok(AudioDecodeResult {
            sink,
            samples: 0,
            has_audio: false,
        });
    };
    if duration == 0 {
        check_cancelled(&cancelled)?;
        sink.finish()?;
        return Ok(AudioDecodeResult {
            sink,
            samples: 0,
            has_audio: true,
        });
    }
    // FFmpeg demux.c adds the container origin to -ss, but subtracts the
    // original target from packets even when its DTS seek heuristic is used.
    let origin = unsafe { (*input.as_ptr()).start_time };
    let target = start
        .checked_add(if origin == ffi::AV_NOPTS_VALUE {
            0
        } else {
            origin
        })
        .ok_or("音频定位时间溢出")?;
    let needs_dts_preroll =
        unsafe { (*input.format().as_ptr()).flags & ffi::AVFMT_SEEK_TO_PTS == 0 }
            && input
                .streams()
                .any(|stream| unsafe { (*stream.parameters().as_ptr()).video_delay != 0 });
    let seek = if needs_dts_preroll {
        target
            .checked_sub(3 * i64::from(ffi::AV_TIME_BASE) / 23)
            .ok_or("音频定位时间溢出")?
    } else {
        target
    };
    // As in the CLI, an unsuccessful seek is not by itself fatal. Subsequent
    // accurate trimming can still produce an empty result or read an input
    // whose demuxer cannot seek. Native I/O errors remain errors below.
    let _seek_result = input.seek(seek, ..seek);
    check_cancelled(&cancelled)?;
    let stream = input.stream(index).ok_or("找不到音轨")?;
    let packet_time_base: ffi::AVRational = stream.time_base().into();
    if packet_time_base.num <= 0 || packet_time_base.den <= 0 {
        return Err("音频流时基无效".into());
    }
    let context = codec::context::Context::from_parameters(stream.parameters())
        .map_err(|error| native_error("初始化音频解码器失败", error))?;
    let mut decoder = context.decoder();
    decoder.set_packet_time_base(stream.time_base());
    let mut decoder = decoder
        .audio()
        .map_err(|error| native_error("打开音频解码器失败", error))?;
    let offset = unsafe {
        ffi::av_rescale_q(
            target.checked_neg().ok_or("音频定位时间溢出")?,
            MICROS,
            packet_time_base,
        )
    };
    let mut output = RangeOutput::new(rate, duration, sink, cancelled.clone());
    decode_frames(
        &mut input,
        index,
        &mut decoder,
        &cancelled,
        |packet: &mut Packet| {
            for (value, pts) in [(packet.pts(), true), (packet.dts(), false)] {
                if let Some(value) = value {
                    let value = value.checked_add(offset).ok_or("音频时间戳溢出")?;
                    if pts {
                        packet.set_pts(Some(value));
                    } else {
                        packet.set_dts(Some(value));
                    }
                }
            }
            Ok(())
        },
        |frame| output.frame(frame, packet_time_base),
    )?;
    output.finish()
}

pub(crate) struct AudioClock {
    base: ffi::AVRational,
    rate: i32,
    last: i64,
    duration: i64,
    delta: i64,
}

impl AudioClock {
    pub(crate) fn new() -> Self {
        Self {
            base: ffi::AVRational { num: 1, den: 1 },
            rate: 0,
            last: ffi::AV_NOPTS_VALUE,
            duration: 0,
            delta: ffi::AV_NOPTS_VALUE,
        }
    }

    /// Same rescale state as fftools/ffmpeg_dec.c audio_ts_process (6.1/9).
    /// In particular, FLV's millisecond timestamps need sample-grid recovery.
    pub(crate) fn normalize(
        &mut self,
        frame: &mut frame::Audio,
        packet_base: ffi::AVRational,
    ) -> Result<(), String> {
        let rate = frame.rate() as i32;
        if rate <= 0 {
            return Err("音频帧缺少采样率".into());
        }
        let sample_base = ffi::AVRational { num: 1, den: rate };
        let mut frame_base = packet_base;
        if self.rate != rate {
            let gcd = unsafe { ffi::av_gcd(i64::from(self.base.den), i64::from(rate)) };
            let previous = i64::from(self.base.den) / gcd;
            let denominator = if previous >= i64::from(i32::MAX) / i64::from(rate) {
                28_224_000
            } else {
                (previous * i64::from(rate)) as i32
            };
            let mut next_base = ffi::AVRational {
                num: 1,
                den: denominator,
            };
            if packet_base.num == 1
                && packet_base.den > denominator
                && packet_base.den % denominator == 0
            {
                next_base = packet_base;
            }
            unsafe {
                if self.last != ffi::AV_NOPTS_VALUE {
                    self.last = ffi::av_rescale_q(self.last, self.base, next_base);
                }
                self.duration = ffi::av_rescale_q(self.duration, self.base, next_base);
            }
            self.base = next_base;
            self.rate = rate;
        }
        let predicted = if self.last == ffi::AV_NOPTS_VALUE {
            0
        } else {
            self.last
                .checked_add(self.duration)
                .ok_or("音频时间戳溢出")?
        };
        let pts = match frame.pts() {
            None => {
                frame_base = self.base;
                predicted
            }
            Some(pts) => {
                if self.last != ffi::AV_NOPTS_VALUE
                    && pts
                        > unsafe {
                            ffi::av_rescale_q_rnd(
                                predicted,
                                self.base,
                                frame_base,
                                ffi::AVRounding::AV_ROUND_UP,
                            )
                        }
                {
                    self.delta = ffi::AV_NOPTS_VALUE;
                }
                pts
            }
        };
        unsafe {
            self.last = ffi::av_rescale_delta(
                frame_base,
                pts,
                self.base,
                frame.samples() as i32,
                &mut self.delta,
                self.base,
            );
            self.duration = ffi::av_rescale_q(frame.samples() as i64, sample_base, self.base);
            frame.set_pts(Some(ffi::av_rescale_q(self.last, self.base, sample_base)));
            (*frame.as_mut_ptr()).duration = frame.samples() as i64;
            (*frame.as_mut_ptr()).time_base = sample_base;
        }
        Ok(())
    }
}

struct RangeOutput<S> {
    sink: S,
    rate: u32,
    duration: i64,
    samples: u64,
    graph: Option<filter::Graph>,
    format: Option<AudioFormat>,
    clock: AudioClock,
    complete: bool,
    cancelled: Arc<AtomicBool>,
}

impl<S: AudioSink> RangeOutput<S> {
    fn new(rate: u32, duration: i64, sink: S, cancelled: Arc<AtomicBool>) -> Self {
        Self {
            sink,
            rate,
            duration,
            samples: 0,
            graph: None,
            format: None,
            clock: AudioClock::new(),
            complete: false,
            cancelled,
        }
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
            // Like ffmpeg_filter.c's input reconfiguration, drain all ready
            // frames (done after every submission) and replace the old graph
            // without sending EOF. Flushing would invent an extra SWR tail.
            // Keep the decoder clock and the encoder's absolute time cutoff.
            self.graph = None;
            self.format = None;
        }
        if self.graph.is_none() {
            self.graph = Some(make_graph(frame, &format, self.rate, self.duration)?);
            self.format = Some(format);
        }
        // Own/copy unspecified layouts before submitting, just as the CLI
        // guesses their channel layout before configuring its input filter.
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
        let mut input = self
            .graph
            .as_mut()
            .unwrap()
            .get("in")
            .ok_or("音频滤镜输入不存在")?;
        let result = unsafe {
            ffi::av_buffersrc_add_frame_flags(
                input.as_mut_ptr(),
                frame.as_mut_ptr(),
                ffi::AV_BUFFERSRC_FLAG_KEEP_REF as i32,
            )
        };
        if result < 0 && result != ffi::AVERROR_EOF {
            return Err(native_error(
                "提交音频滤镜失败",
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
            let graph = self.graph.as_mut().ok_or("音频滤镜不存在")?;
            let mut output = graph.get("out").ok_or("音频滤镜输出不存在")?;
            match output.sink().frame(&mut frame) {
                Ok(()) => (),
                Err(ffmpeg::Error::Eof) => {
                    self.complete = true;
                    break;
                }
                Err(error) if again(error) => break,
                Err(error) => return Err(native_error("读取音频滤镜失败", error)),
            }
            let pts = frame.pts().ok_or("音频滤镜未返回时间戳")?;
            let base: ffi::AVRational = output.sink().time_base().into();
            // fftools/ffmpeg_enc.c check_recording_time rejects a frame at or
            // after -t; earlier frames are encoded whole, including their tail.
            if unsafe { ffi::av_compare_ts(pts, base, self.duration, MICROS) } >= 0 {
                self.complete = true;
                break;
            }
            if frame.samples() == 0 {
                continue;
            }
            let samples = frame.plane::<i16>(0);
            if !samples.is_empty() {
                self.sink.write(samples)?;
                self.samples = self
                    .samples
                    .checked_add(samples.len() as u64)
                    .ok_or("音频样本计数溢出")?;
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Result<AudioDecodeResult<S>, String> {
        if !self.complete {
            if let Some(graph) = self.graph.as_mut() {
                graph
                    .get("in")
                    .ok_or("音频滤镜输入不存在")?
                    .source()
                    .flush()
                    .map_err(|error| native_error("结束音频滤镜失败", error))?;
                self.pull()?;
            }
        }
        check_cancelled(&self.cancelled)?;
        self.sink.finish()?;
        Ok(AudioDecodeResult {
            sink: self.sink,
            samples: self.samples,
            has_audio: true,
        })
    }
}

fn make_graph(
    frame: &frame::Audio,
    format: &AudioFormat,
    rate: u32,
    duration: i64,
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
    let input = filter::find("abuffer").ok_or("FFmpeg 缺少 abuffer 滤镜")?;
    let output = filter::find("abuffersink").ok_or("FFmpeg 缺少 abuffersink 滤镜")?;
    graph
        .add(
            &input,
            "in",
            &format!(
                "time_base=1/{0}:sample_rate={0}:sample_fmt={1}:channel_layout={layout}",
                frame.rate(),
                frame.format().name()
            ),
        )
        .map_err(|error| native_error("创建音频滤镜输入失败", error))?;
    graph
        .add(&output, "out", "")
        .map_err(|error| native_error("创建音频滤镜输出失败", error))?;
    // ffmpeg_filter.c inserts input accurate-seek trim before format
    // negotiation, and output duration trim after format negotiation. Duration
    // is parsed as integer microseconds to retain CLI sub-sample rounding.
    let spec = format!("atrim=start_pts=0,aformat=sample_fmts=s16:sample_rates={rate}:channel_layouts=mono,atrim=durationi={duration}us");
    graph
        .output("in", 0)
        .and_then(|parser| parser.input("out", 0))
        .and_then(|parser| parser.parse(&spec))
        .map_err(|error| native_error("创建音频范围滤镜失败", error))?;
    graph
        .validate()
        .map_err(|error| native_error("配置音频范围滤镜失败", error))?;
    Ok(graph)
}
