use super::{
    util::{self, Device, Limits},
    video_clock::VideoClock,
    video_encoder::VideoEncoderStage,
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
            matrix: frame
                .side_data(frame::side_data::Type::DisplayMatrix)
                .and_then(|side| display_matrix(side.data())),
        }
    }
}

pub(super) struct VideoStage {
    decoder: Option<codec::decoder::Video>,
    encoding: VideoEncoderStage,
    options: VideoEncodeOptions,
    limits: Limits,
    graph: Option<filter::Graph>,
    input_format: Option<InputFormat>,
    clock: VideoClock,
    base: Rational,
    rate: Rational,
    rotation: Option<[i32; 9]>,
    device: Option<Device>,
    done: bool,
    cancelled: Arc<AtomicBool>,
}

impl VideoStage {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        parameters: codec::Parameters,
        source: &super::MediaInput,
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
        let decoder = util::open_decoder(parameters, base, source)?
            .video()
            .map_err(|e| util::error("打开输入解码器失败", e))?;
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
        let encoding = VideoEncoderStage::new(
            options.clone(),
            limits,
            global_header,
            mux_flags,
            preserve_initial_gap,
            avi_auto,
            cancelled.clone(),
        )?;
        let device = options
            .hardware_device
            .as_ref()
            .map(Device::new)
            .transpose()?;
        Ok(Self {
            decoder,
            encoding,
            options,
            limits,
            graph: None,
            input_format: None,
            clock: VideoClock::new(base, average_rate, decoder_rate, timestamps_unreliable),
            base,
            rate,
            rotation,
            device,
            done: false,
            cancelled,
        })
    }
    pub(super) fn ready(&self) -> bool {
        self.encoding.ready()
    }
    pub(super) fn done(&self) -> bool {
        self.done || self.encoding.done()
    }
    pub(super) fn frames(&self) -> u64 {
        self.encoding.frames()
    }
    pub(super) fn parameters(&self) -> Option<codec::Parameters> {
        self.encoding.parameters()
    }
    pub(super) fn encoder_base(&self) -> Rational {
        self.encoding.encoder_base()
    }
    pub(super) fn frame_rate(&self) -> Rational {
        self.encoding.frame_rate()
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
        if let Some((width, height)) = self.encoding.size() {
            if self.device.is_none() {
                filters.push(format!("scale={width}:{height}"));
            }
        }
        filters.push(self.encoding.format_filter()?);
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
        self.encoding
            .open_from_sink(&mut sink, frame, rate, filter_base, self.device.as_ref())?;
        self.graph = Some(graph);
        self.input_format = Some(InputFormat::of(frame));
        Ok(())
    }

    pub(super) fn packet(
        &mut self,
        packet: &Packet,
        output: &mut EncodedPacketSink<'_>,
    ) -> Result<(), String> {
        if self.done() {
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
        while !self.done() {
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
            self.encoding
                .submit_filtered(frame, sink.sink().time_base(), packets)?;
        }
        Ok(())
    }
    pub(super) fn finish(&mut self, output: &mut EncodedPacketSink<'_>) -> Result<(), String> {
        if !self.done() {
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
            if !self.done() {
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
        self.encoding.finish(output)?;
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
        if stage.frames() != 3 {
            return Err("硬件探测没有完成三帧编码".into());
        }
        Ok(())
    }
}
