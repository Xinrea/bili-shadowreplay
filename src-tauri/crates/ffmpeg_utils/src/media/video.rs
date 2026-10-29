use super::{
    util::{self, Device, Limits},
    video_clock::{FrameSync, VideoClock},
    EncodedPacketSink, FrameRatePolicy, VideoEncodeOptions,
};
use crate::video_frame::{display_matrix, filter_has_option, rotation_filters};
use ffmpeg_next::{self as ffmpeg, codec, ffi, filter, format, frame, Packet, Rational};
use std::sync::{atomic::AtomicBool, Arc};

#[derive(Clone, Copy, PartialEq)]
struct InputFormat {
    width: u32,
    height: u32,
    format: format::Pixel,
    sar: Rational,
    space: i32,
    range: i32,
    matrix: Option<[i32; 9]>,
}
impl InputFormat {
    fn of(frame: &frame::Video) -> Self {
        Self {
            width: frame.width(),
            height: frame.height(),
            format: frame.format(),
            sar: frame.aspect_ratio(),
            space: unsafe { (*frame.as_ptr()).colorspace as i32 },
            range: unsafe { (*frame.as_ptr()).color_range as i32 },
            matrix: frame.side_data(frame::side_data::Type::DisplayMatrix).and_then(|side| display_matrix(side.data())),
        }
    }
}

pub(super) struct VideoStage {
    decoder: Option<codec::decoder::Video>,
    encoder: Option<codec::encoder::video::Encoder>,
    codec: codec::Video,
    options: VideoEncodeOptions,
    limits: Limits,
    graph: Option<filter::Graph>,
    input_format: Option<InputFormat>,
    clock: VideoClock,
    sync: Option<FrameSync>,
    base: Rational,
    rate: Rational,
    rotation: Option<[i32; 9]>,
    device: Option<Device>,
    global_header: bool,
    mux_flags: i32,
    preserve_initial_gap: bool,
    avi_auto: bool,
    done: bool,
    cancelled: Arc<AtomicBool>,
    pub frames: u64,
}

impl VideoStage {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        parameters: codec::Parameters,
        base: Rational,
        guessed_rate: Rational,
        average_rate: Rational,
        rotation: Option<[i32; 9]>,
        options: VideoEncodeOptions,
        limits: Limits,
        global_header: bool,
        mux_flags: i32,
        preserve_initial_gap: bool,
        avi_auto: bool,
        timestamps_unreliable: bool,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let mut decoder = codec::Context::from_parameters(parameters)
            .map_err(|e| util::error("创建视频解码器失败", e))?
            .decoder();
        decoder.set_packet_time_base(base);
        let decoder = decoder
            .video()
            .map_err(|e| util::error("打开视频解码器失败", e))?;
        let decoder_rate = decoder.frame_rate().unwrap_or(Rational(0, 1));
        let stage = Self::create(
            Some(decoder),
            base,
            guessed_rate,
            average_rate,
            decoder_rate,
            rotation,
            options,
            limits,
            global_header,
            mux_flags,
            preserve_initial_gap,
            avi_auto,
            timestamps_unreliable,
            cancelled,
        )?;
        Ok(stage)
    }
    #[allow(clippy::too_many_arguments)]
    fn create(
        decoder: Option<codec::decoder::Video>,
        base: Rational,
        rate: Rational,
        average_rate: Rational,
        decoder_rate: Rational,
        rotation: Option<[i32; 9]>,
        options: VideoEncodeOptions,
        limits: Limits,
        global_header: bool,
        mux_flags: i32,
        preserve_initial_gap: bool,
        avi_auto: bool,
        timestamps_unreliable: bool,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let codec = ffmpeg::encoder::find_by_name(&options.encoder)
            .ok_or_else(|| format!("找不到视频编码器 {}", options.encoder))?
            .video()
            .map_err(|e| util::error("视频编码器类型错误", e))?;
        let device = options
            .hardware_device
            .as_ref()
            .map(Device::new)
            .transpose()?;
        Ok(Self {
            decoder,
            encoder: None,
            codec,
            options,
            limits,
            graph: None,
            input_format: None,
            clock: VideoClock::new(base, average_rate, decoder_rate, timestamps_unreliable),
            sync: None,
            base,
            rate,
            rotation,
            device,
            global_header,
            mux_flags,
            preserve_initial_gap,
            avi_auto,
            done: false,
            cancelled,
            frames: 0,
        })
    }
    pub(super) fn ready(&self) -> bool {
        self.encoder.is_some()
    }
    pub(super) fn done(&self) -> bool {
        self.done
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
    pub(super) fn autorotate(&self) -> bool {
        self.options.autorotate
    }

    fn configure(&mut self, frame: &frame::Video) -> Result<(), String> {
        let mut graph = filter::Graph::new();
        unsafe {
            (*graph.as_mut_ptr()).nb_threads = 1;
        }
        let source = filter::find("buffer").ok_or("缺少 buffer 滤镜")?;
        let sar = if frame.aspect_ratio().numerator() > 0 && frame.aspect_ratio().denominator() > 0
        {
            frame.aspect_ratio()
        } else {
            Rational(1, 1)
        };
        let mut args = format!(
            "video_size={}x{}:pix_fmt={}:time_base={}/{}:pixel_aspect={}/{}",
            frame.width(),
            frame.height(),
            unsafe { (*frame.as_ptr()).format },
            self.base.numerator(),
            self.base.denominator(),
            sar.numerator(),
            sar.denominator()
        );
        if self.rate.numerator() > 0 && self.rate.denominator() > 0 {
            args.push_str(&format!(
                ":frame_rate={}/{}",
                self.rate.numerator(),
                self.rate.denominator()
            ));
        }
        if filter_has_option(&source, c"colorspace") {
            args.push_str(&format!(":colorspace={}", unsafe {
                (*frame.as_ptr()).colorspace as i32
            }));
        }
        if filter_has_option(&source, c"range") {
            args.push_str(&format!(":range={}", unsafe {
                (*frame.as_ptr()).color_range as i32
            }));
        }
        graph
            .add(&source, "in", &args)
            .map_err(|e| util::error("创建视频滤镜输入失败", e))?;
        graph
            .add(
                &filter::find("buffersink").ok_or("缺少 buffersink 滤镜")?,
                "out",
                "",
            )
            .map_err(|e| util::error("创建视频滤镜输出失败", e))?;
        let rotation = frame
            .side_data(frame::side_data::Type::DisplayMatrix)
            .and_then(|d| display_matrix(d.data()))
            .or(self.rotation);
        let mut filters = if self.options.autorotate {
            rotation_filters(rotation)
        } else {
            Vec::new()
        };
        if let Some(trim) = self.limits.trim(false, true) {
            filters.push(trim);
        }
        if let Some(filter) = self
            .options
            .filter
            .as_ref()
            .filter(|s| !s.trim().is_empty())
        {
            filters.push(filter.clone());
        }
        let formats = if let Some(encoder) = &self.encoder {
            if self.device.is_none() {
                filters.push(format!("scale={}:{}", encoder.width(), encoder.height()));
            }
            encoder
                .format()
                .descriptor()
                .ok_or("视频编码格式无效")?
                .name()
                .to_owned()
        } else {
            self.codec
                .formats()
                .ok_or("视频编码器没有像素格式列表")?
                .map(|format| {
                    format
                        .descriptor()
                        .map(|d| d.name().to_owned())
                        .ok_or("视频像素格式无效")
                })
                .collect::<Result<Vec<_>, _>>()?
                .join("|")
        };
        filters.push(format!("format=pix_fmts={formats}"));
        if let Some(trim) = self.limits.trim(false, false) {
            filters.push(trim);
        }
        graph
            .output("in", 0)
            .and_then(|p| p.input("out", 0))
            .and_then(|p| p.parse(&filters.join(",")))
            .map_err(|e| util::error("创建视频滤镜图失败", e))?;
        if let Some(device) = &self.device {
            device.attach(&mut graph)?;
        }
        graph
            .validate()
            .map_err(|e| util::error("配置视频滤镜图失败", e))?;
        let mut sink = graph.get("out").ok_or("视频滤镜输出不存在")?;
        let filter_base: Rational = sink.sink().time_base();
        let rate = match self.options.frame_rate {
            FrameRatePolicy::Constant { num, den } => Rational(num as i32, den as i32),
            _ => {
                let rate = unsafe { ffi::av_buffersink_get_frame_rate(sink.as_mut_ptr()) };
                if rate.num > 0 && rate.den > 0 {
                    rate.into()
                } else if self.rate.numerator() > 0 && self.rate.denominator() > 0 {
                    self.rate
                } else {
                    Rational(25, 1)
                }
            }
        };
        if self.encoder.is_none() {
            let pixel_id = unsafe { ffi::av_buffersink_get_format(sink.as_mut_ptr()) };
            let pixel = self
                .codec
                .formats()
                .and_then(|mut formats| {
                    formats.find(|format| ffi::AVPixelFormat::from(*format) as i32 == pixel_id)
                })
                .ok_or("滤镜输出不是编码器支持的视频格式")?;
            let mut encoder = codec::Context::new_with_codec(*self.codec)
                .encoder()
                .video()
                .map_err(|e| util::error("创建视频编码器失败", e))?;
            encoder.set_width(unsafe { ffi::av_buffersink_get_w(sink.as_mut_ptr()) } as u32);
            encoder.set_height(unsafe { ffi::av_buffersink_get_h(sink.as_mut_ptr()) } as u32);
            encoder.set_format(pixel);
            encoder.set_time_base(rate.invert());
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
                raw.sample_aspect_ratio =
                    ffi::av_buffersink_get_sample_aspect_ratio(sink.as_mut_ptr());
                raw.colorspace = (*frame.as_ptr()).colorspace;
                raw.color_range = (*frame.as_ptr()).color_range;
                raw.color_primaries = (*frame.as_ptr()).color_primaries;
                raw.color_trc = (*frame.as_ptr()).color_trc;
                raw.chroma_sample_location = (*frame.as_ptr()).chroma_location;
                if let Some(value) = self.options.buffer_size {
                    raw.rc_buffer_size = value as i32;
                }
                if let Some(value) = self.options.threads {
                    raw.thread_count = value as i32;
                }
                if let Some(device) = &self.device {
                    raw.hw_device_ctx = ffi::av_buffer_ref(device.raw);
                    if raw.hw_device_ctx.is_null() {
                        return Err("复制视频硬件设备失败".into());
                    }
                }
                let frames = ffi::av_buffersink_get_hw_frames_ctx(sink.as_mut_ptr());
                if !frames.is_null() {
                    raw.hw_frames_ctx = ffi::av_buffer_ref(frames);
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
        self.graph = Some(graph);
        self.input_format = Some(InputFormat::of(frame));
        Ok(())
    }

    pub(super) fn packet(
        &mut self,
        packet: &Packet,
        output: &mut EncodedPacketSink<'_>,
    ) -> Result<(), String> {
        if self.done {
            return Ok(());
        }
        match self
            .decoder
            .as_mut()
            .ok_or("视频探测没有输入解码器")?
            .send_packet(packet)
        {
            Ok(()) => (),
            Err(e) if util::again(e) => {
                self.decode(output)?;
                self.decoder
                    .as_mut()
                    .unwrap()
                    .send_packet(packet)
                    .map_err(|e| util::error("提交视频包失败", e))?;
            }
            Err(e) => return Err(util::error("提交视频包失败", e)),
        }
        self.decode(output)?;
        Ok(())
    }
    fn decode(&mut self, packets: &mut EncodedPacketSink<'_>) -> Result<(), String> {
        let mut frame = frame::Video::empty();
        while !self.done {
            util::check(&self.cancelled)?;
            match self.decoder.as_mut().unwrap().receive_frame(&mut frame) {
                Ok(()) => (),
                Err(ffmpeg::Error::Eof) => break,
                Err(e) if util::again(e) => break,
                Err(e) => return Err(util::error("解码视频失败", e)),
            }
            self.clock.set_decoder_rate(
                self.decoder
                    .as_ref()
                    .unwrap()
                    .frame_rate()
                    .unwrap_or(Rational(0, 1)),
            );
            self.clock.normalize(&mut frame)?;
            self.frame(&mut frame, packets)?;
        }
        Ok(())
    }
    fn frame(
        &mut self,
        frame: &mut frame::Video,
        packets: &mut EncodedPacketSink<'_>,
    ) -> Result<(), String> {
        if self
            .input_format
            .is_some_and(|format| format != InputFormat::of(frame))
        {
            self.graph = None;
        }
        if self.graph.is_none() {
            self.configure(frame)?;
        }
        if self.options.autorotate {
            frame.remove_side_data(frame::side_data::Type::DisplayMatrix);
        }
        let mut source = self
            .graph
            .as_mut()
            .unwrap()
            .get("in")
            .ok_or("视频滤镜输入不存在")?;
        let result = unsafe {
            ffi::av_buffersrc_add_frame_flags(
                source.as_mut_ptr(),
                frame.as_mut_ptr(),
                ffi::AV_BUFFERSRC_FLAG_KEEP_REF as i32,
            )
        };
        if result < 0 && result != ffi::AVERROR_EOF {
            return Err(util::error("提交视频滤镜失败", ffmpeg::Error::from(result)));
        }
        self.pull(packets)
    }
    fn pull(&mut self, packets: &mut EncodedPacketSink<'_>) -> Result<(), String> {
        loop {
            util::check(&self.cancelled)?;
            let mut frame = frame::Video::empty();
            let mut sink = self
                .graph
                .as_mut()
                .unwrap()
                .get("out")
                .ok_or("视频滤镜输出不存在")?;
            match sink.sink().frame(&mut frame) {
                Ok(()) => (),
                Err(ffmpeg::Error::Eof) => {
                    self.done = true;
                    break;
                }
                Err(e) if util::again(e) => break,
                Err(e) => return Err(util::error("读取视频滤镜输出失败", e)),
            }
            unsafe {
                (*frame.as_mut_ptr()).time_base = sink.sink().time_base().into();
            }
            let batch = self.sync.as_mut().unwrap().push(Some(&frame))?;
            for frame in batch {
                if self.done {
                    break;
                }
                if self.done {
                    break;
                }
                util::check(&self.cancelled)?;
                self.encode(&mut frame?, packets)?;
            }
        }
        Ok(())
    }
    fn encode(
        &mut self,
        frame: &mut frame::Video,
        packets: &mut EncodedPacketSink<'_>,
    ) -> Result<(), String> {
        if !self.limits.encode(
            frame.pts().ok_or("视频编码帧没有时间戳")?,
            self.encoder_base().into(),
        ) {
            self.done = true;
            return Ok(());
        }
        unsafe {
            (*frame.as_mut_ptr()).pict_type = ffi::AVPictureType::AV_PICTURE_TYPE_NONE;
            (*frame.as_mut_ptr()).quality =
                (*self.encoder.as_ref().unwrap().as_ptr()).global_quality;
        }
        match self.encoder.as_mut().unwrap().send_frame(frame) {
            Ok(()) => (),
            Err(e) if util::again(e) => {
                self.drain(packets)?;
                self.encoder
                    .as_mut()
                    .unwrap()
                    .send_frame(frame)
                    .map_err(|e| util::error("提交视频编码帧失败", e))?;
            }
            Err(e) => return Err(util::error("提交视频编码帧失败", e)),
        }
        self.frames = self.frames.checked_add(1).ok_or("视频帧计数溢出")?;
        self.drain(packets)?;
        Ok(())
    }
    fn drain(&mut self, packets: &mut EncodedPacketSink<'_>) -> Result<bool, String> {
        loop {
            util::check(&self.cancelled)?;
            let mut packet = Packet::empty();
            match self.encoder.as_mut().unwrap().receive_packet(&mut packet) {
                Ok(()) => packets(packet, self.encoder_base())?,
                Err(ffmpeg::Error::Eof) => return Ok(true),
                Err(e) if util::again(e) => return Ok(false),
                Err(e) => return Err(util::error("读取视频编码包失败", e)),
            }
        }
    }
    pub(super) fn finish(&mut self, output: &mut EncodedPacketSink<'_>) -> Result<(), String> {
        if !self.done {
            if self.decoder.is_some() {
                match self.decoder.as_mut().unwrap().send_eof() {
                    Ok(()) | Err(ffmpeg::Error::Eof) => (),
                    Err(e) if util::again(e) => {
                        self.decode(output)?;
                        self.decoder
                            .as_mut()
                            .unwrap()
                            .send_eof()
                            .map_err(|e| util::error("结束视频解码失败", e))?;
                    }
                    Err(e) => return Err(util::error("结束视频解码失败", e)),
                }
                self.decode(output)?;
            }
            if !self.done {
                if let Some(graph) = self.graph.as_mut() {
                    graph
                        .get("in")
                        .ok_or("视频滤镜输入不存在")?
                        .source()
                        .flush()
                        .map_err(|e| util::error("结束视频滤镜失败", e))?;
                    self.pull(output)?;
                }
            }
        }
        if self.encoder.is_none() {
            return Err("视频流缺少可配置的编码参数".into());
        }
        if let Some(sync) = self.sync.as_mut() {
            let batch = sync.push(None)?;
            for frame in batch {
                if self.done {
                    break;
                }
                if self.done {
                    break;
                }
                util::check(&self.cancelled)?;
                self.encode(&mut frame?, output)?;
            }
        }
        match self.encoder.as_mut().unwrap().send_eof() {
            Ok(()) | Err(ffmpeg::Error::Eof) => (),
            Err(e) if util::again(e) => {
                self.drain(output)?;
                self.encoder
                    .as_mut()
                    .unwrap()
                    .send_eof()
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

    pub(super) fn probe(
        options: VideoEncodeOptions,
        cancelled: Arc<AtomicBool>,
    ) -> Result<(), String> {
        let limits = Limits {
            input_start: None,
            input_duration: None,
            output_start: 0,
            output_seek: false,
            output_duration: None,
        };
        let mut stage = Self::create(
            None,
            Rational(1, 30),
            Rational(30, 1),
            Rational(30, 1),
            Rational(30, 1),
            None,
            options,
            limits,
            false,
            ffi::AVFMT_NOTIMESTAMPS,
            true,
            false,
            false,
            cancelled,
        )?;
        for index in 0..3 {
            let mut frame = frame::Video::empty();
            frame.set_format(format::Pixel::YUV420P);
            frame.set_width(320);
            frame.set_height(240);
            unsafe {
                (*frame.as_mut_ptr()).time_base = ffi::AVRational { num: 1, den: 30 };
                (*frame.as_mut_ptr()).duration = 1;
                (*frame.as_mut_ptr()).sample_aspect_ratio = ffi::AVRational { num: 1, den: 1 };
                let result = ffi::av_frame_get_buffer(frame.as_mut_ptr(), 32);
                if result < 0 {
                    return Err(util::error(
                        "创建编码探测帧失败",
                        ffmpeg::Error::from(result),
                    ));
                }
            }
            for plane in 0..3 {
                let width = if plane == 0 { 320 } else { 160 };
                let height = if plane == 0 { 240 } else { 120 };
                let stride = frame.stride(plane);
                for y in 0..height {
                    for x in 0..width {
                        frame.data_mut(plane)[y * stride + x] = if plane == 0 {
                            ((x + y + index * 17) % 220 + 16) as u8
                        } else {
                            128
                        };
                    }
                }
            }
            frame.set_pts(Some(index as i64));
            stage.frame(&mut frame, &mut |_, _| Ok(()))?;
        }
        stage.finish(&mut |_, _| Ok(()))?;
        if stage.frames != 3 {
            return Err("硬件探测没有完成三帧编码".into());
        }
        Ok(())
    }
}
