//! Shared filtered-video encoder and FPS conversion, independent of demuxers.
use crate::profiling as perf;
use super::{
    util::{self, Device, Limits},
    video_clock::FrameSync,
    EncodedPacketSink, FrameRatePolicy, VideoEncodeOptions, VideoFrameInfo,
    VideoFrameProcessor, VideoFrameProcessorFactory,
};
use ffmpeg_next::{self as ffmpeg, codec, ffi, filter, frame, Packet, Rational};
use std::{collections::VecDeque, sync::{atomic::AtomicBool, Arc}};
pub(super) struct VideoEncoderStage {
    encoder: Option<codec::encoder::video::Encoder>,
    codec: codec::Video,
    options: VideoEncodeOptions,
    limits: Limits,
    sync: Option<FrameSync>,
    global_header: bool,
    mux_flags: i32,
    preserve_initial_gap: bool,
    avi_auto: bool,
    done: bool,
    cancelled: Arc<AtomicBool>,
    frames: u64,
    processor_factory: Option<Box<dyn VideoFrameProcessorFactory>>,
    processor: Option<Box<dyn VideoFrameProcessor>>,
    pending_processed: VecDeque<(i64, i64)>,
}
impl VideoEncoderStage {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        options: VideoEncodeOptions,
        limits: Limits,
        global_header: bool,
        mux_flags: i32,
        preserve_initial_gap: bool,
        avi_auto: bool,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let codec = ffmpeg::encoder::find_by_name(&options.encoder)
            .ok_or_else(|| format!("找不到视频编码器 {}", options.encoder))?
            .video()
            .map_err(|e| util::error("视频编码器类型错误", e))?;
        Ok(Self {
            encoder: None,
            codec,
            options,
            limits,
            sync: None,
            global_header,
            mux_flags,
            preserve_initial_gap,
            avi_auto,
            done: false,
            cancelled,
            frames: 0,
            processor_factory: None,
            processor: None,
            pending_processed: VecDeque::new(),
        })
    }
    pub(super) fn with_processor_factory(
        mut self,
        factory: Option<Box<dyn VideoFrameProcessorFactory>>,
    ) -> Self {
        self.processor_factory = factory;
        self
    }
    pub(super) fn frames(&self) -> u64 {
        self.frames
    }
    pub(super) fn size(&self) -> Option<(u32, u32)> {
        self.encoder.as_ref().map(|e| (e.width(), e.height()))
    }
    pub(super) fn format_filter(&self) -> Result<String, String> {
        if self.options.frame_processor {
            return Ok("format=pix_fmts=nv12".into());
        }
        let formats = if let Some(encoder) = &self.encoder {
            encoder
                .format()
                .descriptor()
                .ok_or("视频编码格式无效")?
                .name()
                .to_owned()
        } else {
            let Some(formats) = self.codec.formats() else {
                return Ok("null".into());
            };
            formats
                .map(|format| {
                    format
                        .descriptor()
                        .map(|d| d.name().to_owned())
                        .ok_or("视频像素格式无效")
                })
                .collect::<Result<Vec<_>, _>>()?
                .join("|")
        };
        Ok(format!("format=pix_fmts={formats}"))
    }
    pub(super) fn ready(&self) -> bool {
        self.encoder.is_some()
    }
    pub(super) fn parameters(&self) -> Option<codec::Parameters> {
        self.encoder.as_ref().map(codec::Parameters::from)
    }
    pub(super) fn encoder_base(&self) -> Rational {
        self.encoder.as_ref().unwrap().time_base()
    }
    pub(super) fn frame_rate(&self) -> Rational {
        unsafe { (*self.encoder.as_ref().unwrap().as_ptr()).framerate.into() }
    }
    pub(super) fn done(&self) -> bool {
        self.done
    }
    pub(super) fn open_from_sink(
        &mut self,
        sink: &mut filter::Context,
        frame: &frame::Video,
        rate: Rational,
        filter_base: Rational,
        device: Option<&Device>,
    ) -> Result<(), String> {
        let mut rate = match self.options.frame_rate {
            FrameRatePolicy::Constant { num, den } => Rational(num as i32, den as i32),
            _ => rate,
        };
        if self.encoder.is_none() {
            let is_cfr = match self.options.frame_rate {
                FrameRatePolicy::Constant { .. } => true,
                FrameRatePolicy::Auto => {
                    !self.avi_auto && self.mux_flags & ffi::AVFMT_VARIABLE_FPS == 0
                }
                FrameRatePolicy::Variable | FrameRatePolicy::Passthrough => false,
            };
            if (rate.numerator() <= 0 || rate.denominator() <= 0) && is_cfr {
                rate = Rational(25, 1);
            }
            let encoder_base = if rate.numerator() > 0 && rate.denominator() > 0 {
                rate.invert()
            } else {
                filter_base
            };
            if self.options.frame_processor {
                let factory = self.processor_factory.take().ok_or("视频帧处理器缺少工厂")?;
                self.processor = Some(factory.create(
                    VideoFrameInfo {
                        width: unsafe { ffi::av_buffersink_get_w(sink.as_mut_ptr()) } as u32,
                        height: unsafe { ffi::av_buffersink_get_h(sink.as_mut_ptr()) } as u32,
                        frame_rate: rate,
                        time_base: encoder_base,
                    },
                    self.cancelled.clone(),
                )?);
            }
            let processed_template = self.processor.as_ref()
                .map(|processor| processor.output_template()).transpose()?;
            let template = processed_template.as_ref().unwrap_or(frame);
            let pixel_id = if processed_template.is_some() {
                unsafe { (*template.as_ptr()).format }
            } else {
                unsafe { ffi::av_buffersink_get_format(sink.as_mut_ptr()) }
            };
            let pixel = if let Some(mut formats) = self.codec.formats() {
                formats
                    .find(|format| ffi::AVPixelFormat::from(*format) as i32 == pixel_id)
                    .ok_or("滤镜输出不是编码器支持的视频格式")?
            } else {
                // A null codec format list means unrestricted (e.g. rawvideo),
                // not an encoder without usable formats. The sink's format is
                // already validated by libavfilter.
                if pixel_id < 0 {
                    return Err("滤镜输出视频格式无效".into());
                }
                let raw: ffi::AVPixelFormat = unsafe { std::mem::transmute(pixel_id) };
                ffmpeg::format::Pixel::from(raw)
            };
            let mut encoder = codec::Context::new_with_codec(*self.codec)
                .encoder()
                .video()
                .map_err(|e| util::error("创建视频编码器失败", e))?;
            let (width, height) = if processed_template.is_some() {
                (template.width(), template.height())
            } else {
                (unsafe { ffi::av_buffersink_get_w(sink.as_mut_ptr()) } as u32,
                 unsafe { ffi::av_buffersink_get_h(sink.as_mut_ptr()) } as u32)
            };
            if width == 0 || height == 0 {
                return Err("视频帧处理器返回无效输出尺寸".into());
            }
            encoder.set_width(width);
            encoder.set_height(height);
            encoder.set_format(pixel);
            encoder.set_time_base(encoder_base);
            encoder.set_frame_rate(Some(rate));
            if let Some(value) = self.options.bit_rate {
                encoder.set_bit_rate(value as usize);
            }
            if let Some(value) = self.options.max_rate {
                encoder.set_max_bit_rate(value as usize);
            }
            if let Some(value) = self.options.gop {
                encoder.set_gop(value);
            }
            if self.global_header {
                unsafe {
                    (*encoder.as_mut_ptr()).flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
                }
            }
            unsafe {
                let raw = &mut *encoder.as_mut_ptr();
                raw.flags |= ffi::AV_CODEC_FLAG_FRAME_DURATION as i32;
                raw.sample_aspect_ratio = if processed_template.is_some() {
                    (*template.as_ptr()).sample_aspect_ratio
                } else { ffi::av_buffersink_get_sample_aspect_ratio(sink.as_mut_ptr()) };
                raw.colorspace = (*template.as_ptr()).colorspace;
                raw.color_range = (*template.as_ptr()).color_range;
                raw.color_primaries = (*template.as_ptr()).color_primaries;
                raw.color_trc = (*template.as_ptr()).color_trc;
                raw.chroma_sample_location = (*template.as_ptr()).chroma_location;
                if let Some(value) = self.options.buffer_size {
                    raw.rc_buffer_size = value as i32;
                }
                if let Some(value) = self.options.threads {
                    raw.thread_count = value as i32;
                }
                if let Some(device) = device {
                    raw.hw_device_ctx = ffi::av_buffer_ref(device.raw);
                    if raw.hw_device_ctx.is_null() {
                        return Err("复制视频硬件设备失败".into());
                    }
                }
                let frames = if processed_template.is_some() {
                    (*template.as_ptr()).hw_frames_ctx
                } else { ffi::av_buffersink_get_hw_frames_ctx(sink.as_mut_ptr()) };
                if !frames.is_null() {
                    raw.hw_frames_ctx = ffi::av_buffer_ref(frames);
                    if raw.hw_device_ctx.is_null() {
                        let pool = &*(*frames).data.cast::<ffi::AVHWFramesContext>();
                        raw.hw_device_ctx = ffi::av_buffer_ref(pool.device_ref);
                        if raw.hw_device_ctx.is_null() {
                            return Err("复制全局图硬件设备失败".into());
                        }
                    }
                    if raw.hw_frames_ctx.is_null() {
                        return Err("复制硬件视频帧池失败".into());
                    }
                }
                if let Some(tag) = &self.options.tag {
                    let bytes: [u8; 4] = tag
                        .as_bytes()
                        .try_into()
                        .map_err(|_| "视频编码标签必须为4字节")?;
                    raw.codec_tag = u32::from_le_bytes(bytes);
                }
            }
            util::open_codec(encoder.as_mut(), *self.codec, &self.options.options)?;
            self.encoder = Some(codec::encoder::video::Encoder(encoder));
        }
        if self.sync.is_none() {
            self.sync = Some(
                FrameSync::new(
                    self.options.frame_rate,
                    rate,
                    filter_base,
                    self.encoder_base(),
                    self.mux_flags,
                    self.limits.output_start,
                )?
                .preserve_initial_gap(self.preserve_initial_gap)
                .avi_auto(self.avi_auto),
            );
        }
        Ok(())
    }
    pub(super) fn submit_filtered(
        &mut self,
        mut frame: frame::Video,
        filter_base: Rational,
        packets: &mut EncodedPacketSink<'_>,
    ) -> Result<(), String> {
        if self.done {
            return Ok(());
        }
        unsafe {
            (*frame.as_mut_ptr()).time_base = filter_base.into();
        }
        let batch = self
            .sync
            .as_mut()
            .ok_or("视频编码器未初始化帧率转换")?
            .push(Some(&frame))?;
        for frame in batch {
            if self.done {
                break;
            }
            util::check(&self.cancelled)?;
            self.encode(frame?, packets)?;
        }
        Ok(())
    }
    fn encode(
        &mut self,
        mut frame: frame::Video,
        packets: &mut EncodedPacketSink<'_>,
    ) -> Result<(), String> {
        if !self.limits.encode(
            frame.pts().ok_or("视频编码帧没有时间戳")?,
            self.encoder_base().into(),
        ) {
            self.done = true;
            return Ok(());
        }
        if let Some(processor) = &mut self.processor {
            self.pending_processed.push_back((
                frame.pts().ok_or("视频编码帧没有时间戳")?,
                unsafe { (*frame.as_ptr()).duration },
            ));
            let ready = processor.process(frame)?;
            self.send_processed(ready, packets)?;
            if self.pending_processed.len() > 8 {
                return Err("视频帧处理器缓冲超过八帧".into());
            }
        } else {
            self.send_frame(&mut frame, packets)?;
        }
        self.frames = self.frames.checked_add(1).ok_or("视频帧计数溢出")?;
        perf::video_checkpoint(self.frames);
        Ok(())
    }
    fn send_processed(
        &mut self,
        frames: Vec<frame::Video>,
        packets: &mut EncodedPacketSink<'_>,
    ) -> Result<(), String> {
        for mut frame in frames {
            util::check(&self.cancelled)?;
            let expected = self.pending_processed.pop_front()
                .ok_or("视频帧处理器返回了未提交的帧")?;
            let encoder = self.encoder.as_ref().ok_or("视频编码器尚未创建")?;
            let actual = (frame.pts().ok_or("处理后视频帧缺少时间戳")?,
                          unsafe { (*frame.as_ptr()).duration });
            if actual != expected {
                return Err("视频帧处理器改变了帧顺序、时间戳或时长".into());
            }
            if frame.width() != encoder.width() || frame.height() != encoder.height()
                || frame.format() != encoder.format()
            {
                return Err("视频帧处理器改变了编码输出格式".into());
            }
            unsafe {
                let expected_pool = (*encoder.as_ptr()).hw_frames_ctx;
                let actual_pool = (*frame.as_ptr()).hw_frames_ctx;
                if !expected_pool.is_null() && (actual_pool.is_null()
                    || (*expected_pool).data != (*actual_pool).data)
                {
                    return Err("视频帧处理器返回了不同硬件帧池".into());
                }
                (*frame.as_mut_ptr()).time_base = encoder.time_base().into();
            }
            self.send_frame(&mut frame, packets)?;
        }
        Ok(())
    }
    fn send_frame(
        &mut self,
        frame: &mut frame::Video,
        packets: &mut EncodedPacketSink<'_>,
    ) -> Result<(), String> {
        unsafe {
            (*frame.as_mut_ptr()).pict_type = ffi::AVPictureType::AV_PICTURE_TYPE_NONE;
            (*frame.as_mut_ptr()).quality =
                (*self.encoder.as_ref().unwrap().as_ptr()).global_quality;
        }
        match perf::measure("encode_send", || self.encoder.as_mut().unwrap().send_frame(frame)) {
            Ok(()) => (),
            Err(e) if util::again(e) => {
                self.drain(packets)?;
                perf::measure("encode_send", || self.encoder.as_mut().unwrap().send_frame(frame))
                    .map_err(|e| util::error("提交视频编码帧失败", e))?;
            }
            Err(e) => return Err(util::error("提交视频编码帧失败", e)),
        }
        self.drain(packets)?;
        Ok(())
    }
    fn drain(&mut self, packets: &mut EncodedPacketSink<'_>) -> Result<bool, String> {
        loop {
            util::check(&self.cancelled)?;
            let mut packet = Packet::empty();
            match perf::measure("encode_receive", || self.encoder.as_mut().unwrap().receive_packet(&mut packet)) {
                Ok(()) => packets(packet, self.encoder_base())?,
                Err(ffmpeg::Error::Eof) => return Ok(true),
                Err(e) if util::again(e) => return Ok(false),
                Err(e) => return Err(util::error("读取视频编码包失败", e)),
            }
        }
    }
    pub(super) fn finish(&mut self, output: &mut EncodedPacketSink<'_>) -> Result<(), String> {
        if self.encoder.is_none() {
            return Err("视频流缺少可配置的编码参数".into());
        }
        if let Some(sync) = self.sync.as_mut() {
            let batch = sync.push(None)?;
            for frame in batch {
                if self.done {
                    break;
                }
                util::check(&self.cancelled)?;
                self.encode(frame?, output)?;
            }
        }
        if let Some(processor) = &mut self.processor {
            let ready = processor.flush()?;
            self.send_processed(ready, output)?;
            if !self.pending_processed.is_empty() {
                return Err("视频帧处理器结束后仍有未输出帧".into());
            }
        }
        match perf::measure("encode_eof", || self.encoder.as_mut().unwrap().send_eof()) {
            Ok(()) | Err(ffmpeg::Error::Eof) => (),
            Err(e) if util::again(e) => {
                self.drain(output)?;
                perf::measure("encode_eof", || self.encoder.as_mut().unwrap().send_eof())
                    .map_err(|e| util::error("结束视频编码失败", e))?;
            }
            Err(e) => return Err(util::error("结束视频编码失败", e)),
        }
        if !self.drain(output)? {
            return Err("视频编码器结束后仍要求输入".into());
        }
        self.done = true;
        Ok(())
    }
}
