//! Shared negotiated audio encoder and sample queue for file and graph jobs.
use super::{
    util::{self, Limits},
    AudioEncodeOptions, EncodedPacketSink,
};
use crate::audio::{AudioFormat, AudioFrameQueue};
use ffmpeg_next::{self as ffmpeg, codec, ffi, filter, format, frame, Packet, Rational};
use std::{
    ffi::{c_char, CStr},
    sync::{atomic::AtomicBool, Arc},
};

pub(super) struct AudioEncoderStage {
    encoder: Option<codec::encoder::audio::Encoder>,
    output_format: Option<AudioFormat>,
    codec: codec::Audio,
    options: AudioEncodeOptions,
    limits: Limits,
    queue: AudioFrameQueue,
    global_header: bool,
    done: bool,
    cancelled: Arc<AtomicBool>,
    samples: u64,
    next_pts: i64,
}
impl AudioEncoderStage {
    pub(super) fn new(
        options: AudioEncodeOptions,
        limits: Limits,
        global_header: bool,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let codec = ffmpeg::encoder::find_by_name(&options.encoder)
            .ok_or_else(|| format!("找不到音频编码器 {}", options.encoder))?
            .audio()
            .map_err(|e| util::error("音频编码器类型错误", e))?;
        Ok(Self {
            encoder: None,
            output_format: None,
            codec,
            options,
            limits,
            queue: AudioFrameQueue::default(),
            global_header,
            done: false,
            cancelled,
            samples: 0,
            next_pts: 0,
        })
    }
    pub(super) fn ready(&self) -> bool {
        self.encoder.is_some()
    }
    pub(super) fn done(&self) -> bool {
        self.done
    }
    pub(super) fn samples(&self) -> u64 {
        self.samples
    }
    pub(super) fn parameters(&self) -> Option<codec::Parameters> {
        self.encoder.as_ref().map(codec::Parameters::from)
    }
    pub(super) fn encoder_base(&self) -> Rational {
        self.encoder.as_ref().unwrap().time_base()
    }
    pub(super) fn format_filter(&self) -> Result<String, String> {
        let formats = if let Some(locked) = &self.output_format {
            format!(
                "aformat=sample_fmts={}:sample_rates={}:channel_layouts={}",
                format::Sample::from(locked.sample_format).name(),
                locked.sample_rate,
                layout_name(&locked.layout)?
            )
        } else {
            let formats = self
                .codec
                .formats()
                .ok_or("编码器没有音频格式列表")?
                .map(|f| f.name())
                .collect::<Vec<_>>()
                .join("|");
            let mut spec = format!("aformat=sample_fmts={formats}");
            if let Some(rate) = self.options.sample_rate {
                spec.push_str(&format!(":sample_rates={rate}"));
            } else if let Some(rates) = self.codec.rates() {
                spec.push_str(&format!(
                    ":sample_rates={}",
                    rates.map(|r| r.to_string()).collect::<Vec<_>>().join("|")
                ));
            }
            if let Some(layout) = &self.options.channel_layout {
                spec.push_str(&format!(":channel_layouts={layout}"));
            } else if let Some(layouts) = self.codec.channel_layouts() {
                spec.push_str(&format!(
                    ":channel_layouts={}",
                    layouts
                        .map(|l| format!("0x{:x}", l.bits()))
                        .collect::<Vec<_>>()
                        .join("|")
                ));
            }
            spec
        };
        Ok(formats)
    }
    pub(super) fn open_from_sink(&mut self, sink: &mut filter::Context) -> Result<(), String> {
        if self.encoder.is_none() {
            let format_id = unsafe { ffi::av_buffersink_get_format(sink.as_mut_ptr()) };
            let sample_format = self
                .codec
                .formats()
                .and_then(|mut f| f.find(|f| ffi::AVSampleFormat::from(*f) as i32 == format_id))
                .ok_or("音频输出格式不受编码器支持")?;
            let mut output_format = AudioFormat {
                layout: unsafe { std::mem::zeroed() },
                sample_format: sample_format.into(),
                sample_rate: unsafe { ffi::av_buffersink_get_sample_rate(sink.as_mut_ptr()) },
            };
            let result = unsafe {
                ffi::av_buffersink_get_ch_layout(sink.as_mut_ptr(), &mut output_format.layout)
            };
            if result < 0 {
                return Err(util::error(
                    "读取音频滤镜声道失败",
                    ffmpeg::Error::from(result),
                ));
            }
            let mut encoder = codec::Context::new_with_codec(*self.codec)
                .encoder()
                .audio()
                .map_err(|e| util::error("创建音频编码器失败", e))?;
            encoder.set_rate(output_format.sample_rate);
            encoder.set_format(sample_format);
            encoder.set_time_base((1, output_format.sample_rate));
            unsafe {
                let result = ffi::av_channel_layout_copy(
                    &mut (*encoder.as_mut_ptr()).ch_layout,
                    &output_format.layout,
                );
                if result < 0 {
                    return Err(util::error(
                        "复制编码器声道失败",
                        ffmpeg::Error::from(result),
                    ));
                }
            }
            if let Some(rate) = self.options.bit_rate {
                encoder.set_bit_rate(rate as usize);
            }
            if self.global_header {
                unsafe {
                    (*encoder.as_mut_ptr()).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
                }
            }
            unsafe {
                (*encoder.as_mut_ptr()).flags |= ffi::AV_CODEC_FLAG_FRAME_DURATION as i32;
            }
            util::open_codec(encoder.as_mut(), *self.codec, &self.options.options)?;
            self.encoder = Some(codec::encoder::audio::Encoder(encoder));
            self.output_format = Some(output_format);
        }
        Ok(())
    }
    pub(super) fn submit_filtered(
        &mut self,
        mut frame: frame::Audio,
        filter_base: Rational,
        output: &mut EncodedPacketSink<'_>,
    ) -> Result<(), String> {
        if self.done || frame.samples() == 0 {
            return Ok(());
        }
        util::check(&self.cancelled)?;
        let input_base: ffi::AVRational = filter_base.into();
        let output_base: ffi::AVRational = self.encoder_base().into();
        // fftools' fg_output_frame continues missing timestamps from the
        // preceding filtered frame. apad can produce such frames when its
        // input was empty; valid timestamps still preserve deliberate gaps.
        let shifted = if let Some(pts) = frame.pts() {
            unsafe { ffi::av_rescale_q(pts, input_base, output_base) }
                .checked_sub(unsafe {
                    ffi::av_rescale_q(self.limits.output_start, util::MICROS, output_base)
                })
                .ok_or("音频时间戳溢出")?
        } else {
            self.next_pts
        };
        let sample_rate = i32::try_from(frame.rate())
            .ok()
            .filter(|rate| *rate > 0)
            .ok_or("音频滤镜输出采样率无效")?;
        let duration = unsafe {
            ffi::av_rescale_q(
                frame.samples() as i64,
                ffi::AVRational {
                    num: 1,
                    den: sample_rate,
                },
                output_base,
            )
        };
        self.next_pts = shifted.checked_add(duration).ok_or("音频时间戳溢出")?;
        frame.set_pts(Some(shifted));
        unsafe {
            (*frame.as_mut_ptr()).time_base = output_base;
            (*frame.as_mut_ptr()).duration = duration;
        }
        if self.encoder.as_ref().unwrap().frame_size() == 0 {
            self.encode(&frame, output)?;
        } else {
            self.queue.push(frame)?;
            self.queued(false, output)?;
        }
        Ok(())
    }
    fn queued(&mut self, eof: bool, packets: &mut EncodedPacketSink<'_>) -> Result<(), String> {
        let size = self.encoder.as_ref().unwrap().frame_size() as usize;
        while let Some(frame) = self.queue.pop(size, eof)? {
            self.encode(&frame, packets)?;
        }
        Ok(())
    }
    fn encode(
        &mut self,
        frame: &frame::Audio,
        packets: &mut EncodedPacketSink<'_>,
    ) -> Result<(), String> {
        util::check(&self.cancelled)?;
        if !self.limits.encode(
            frame.pts().ok_or("音频编码帧没有时间戳")?,
            self.encoder_base().into(),
        ) {
            self.done = true;
            return Ok(());
        }
        self.samples = self
            .samples
            .checked_add(frame.samples() as u64)
            .ok_or("音频样本计数溢出")?;
        match self.encoder.as_mut().unwrap().send_frame(frame) {
            Ok(()) => (),
            Err(e) if util::again(e) => {
                self.drain(packets)?;
                self.encoder
                    .as_mut()
                    .unwrap()
                    .send_frame(frame)
                    .map_err(|e| util::error("提交音频编码帧失败", e))?;
            }
            Err(e) => return Err(util::error("提交音频编码帧失败", e)),
        }
        self.drain(packets)?;
        Ok(())
    }
    fn drain(&mut self, output: &mut EncodedPacketSink<'_>) -> Result<bool, String> {
        loop {
            util::check(&self.cancelled)?;
            let mut packet = Packet::empty();
            match self.encoder.as_mut().unwrap().receive_packet(&mut packet) {
                Ok(()) => output(packet, self.encoder_base())?,
                Err(ffmpeg::Error::Eof) => return Ok(true),
                Err(e) if util::again(e) => return Ok(false),
                Err(e) => return Err(util::error("读取音频编码包失败", e)),
            }
        }
    }
    pub(super) fn finish(&mut self, output: &mut EncodedPacketSink<'_>) -> Result<(), String> {
        if self.encoder.is_none() {
            return Err("音频流缺少可配置的编码参数".into());
        }
        self.queued(true, output)?;
        match self.encoder.as_mut().unwrap().send_eof() {
            Ok(()) | Err(ffmpeg::Error::Eof) => (),
            Err(e) if util::again(e) => {
                self.drain(output)?;
                self.encoder
                    .as_mut()
                    .unwrap()
                    .send_eof()
                    .map_err(|e| util::error("结束音频编码失败", e))?;
            }
            Err(e) => return Err(util::error("结束音频编码失败", e)),
        }
        if !self.drain(output)? {
            return Err("音频编码器结束后仍要求输入".into());
        }
        self.done = true;
        Ok(())
    }
}

pub(super) fn layout_name(layout: &ffi::AVChannelLayout) -> Result<String, String> {
    let mut buffer = vec![0 as c_char; 256];
    let size =
        unsafe { ffi::av_channel_layout_describe(layout, buffer.as_mut_ptr(), buffer.len()) };
    if size < 0 {
        return Err("音频布局无效".into());
    }
    if size as usize >= buffer.len() {
        buffer.resize(size as usize + 1, 0);
        let result =
            unsafe { ffi::av_channel_layout_describe(layout, buffer.as_mut_ptr(), buffer.len()) };
        if result < 0 {
            return Err("音频布局无效".into());
        }
    }
    unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_str()
        .map(str::to_owned)
        .map_err(|e| e.to_string())
}
