use super::{
    util::{self, Limits},
    AudioEncodeOptions, EncodedPacketSink,
};
use crate::audio::{AudioClock, AudioFormat, AudioFrameQueue};
use ffmpeg_next::{self as ffmpeg, codec, ffi, filter, format, frame, Packet, Rational};
use std::{
    ffi::{c_char, CStr},
    sync::{atomic::AtomicBool, Arc},
};

pub(super) struct AudioStage {
    decoder: codec::decoder::Audio,
    encoder: Option<codec::encoder::audio::Encoder>,
    graph: Option<filter::Graph>,
    input_format: Option<AudioFormat>,
    output_format: Option<AudioFormat>,
    codec: codec::Audio,
    options: AudioEncodeOptions,
    base: Rational,
    limits: Limits,
    clock: AudioClock,
    queue: AudioFrameQueue,
    global_header: bool,
    done: bool,
    cancelled: Arc<AtomicBool>,
    pub samples: u64,
}

impl AudioStage {
    pub(super) fn new(
        parameters: codec::Parameters,
        base: Rational,
        options: AudioEncodeOptions,
        limits: Limits,
        global_header: bool,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let codec = ffmpeg::encoder::find_by_name(&options.encoder)
            .ok_or_else(|| format!("找不到音频编码器 {}", options.encoder))?
            .audio()
            .map_err(|e| util::error("音频编码器类型错误", e))?;
        let mut decoder = codec::Context::from_parameters(parameters)
            .map_err(|e| util::error("创建音频解码器失败", e))?
            .decoder();
        decoder.set_packet_time_base(base);
        let decoder = decoder
            .audio()
            .map_err(|e| util::error("打开音频解码器失败", e))?;
        let mut output = Self {
            decoder,
            encoder: None,
            graph: None,
            input_format: None,
            output_format: None,
            codec,
            options,
            base,
            limits,
            clock: AudioClock::new(),
            queue: AudioFrameQueue::default(),
            global_header,
            done: false,
            cancelled,
            samples: 0,
        };
        if output.decoder.rate() != 0 && output.decoder.format() != format::Sample::None {
            let mut frame = frame::Audio::empty();
            frame.set_rate(output.decoder.rate());
            frame.set_format(output.decoder.format());
            let result = unsafe {
                ffi::av_channel_layout_copy(
                    &mut (*frame.as_mut_ptr()).ch_layout,
                    &(*output.decoder.as_ptr()).ch_layout,
                )
            };
            if result < 0 {
                return Err(util::error(
                    "复制解码器声道失败",
                    ffmpeg::Error::from(result),
                ));
            }
            if unsafe { (*frame.as_ptr()).ch_layout.nb_channels > 0 } {
                output.configure(&frame)?;
            }
        }
        Ok(output)
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

    fn configure(&mut self, frame: &frame::Audio) -> Result<(), String> {
        let input = AudioFormat::from_frame(frame)?;
        let layout = layout_name(&input.layout)?;
        let mut graph = filter::Graph::new();
        unsafe {
            (*graph.as_mut_ptr()).nb_threads = 1;
        }
        graph
            .add(
                &filter::find("abuffer").ok_or("缺少 abuffer 滤镜")?,
                "in",
                &format!(
                    "time_base=1/{0}:sample_rate={0}:sample_fmt={1}:channel_layout={layout}",
                    frame.rate(),
                    frame.format().name()
                ),
            )
            .map_err(|e| util::error("创建音频滤镜输入失败", e))?;
        graph
            .add(
                &filter::find("abuffersink").ok_or("缺少 abuffersink 滤镜")?,
                "out",
                "",
            )
            .map_err(|e| util::error("创建音频滤镜输出失败", e))?;
        let mut filters = Vec::new();
        if let Some(trim) = self.limits.trim(true, true) {
            filters.push(trim);
        }
        filters.push(
            self.options
                .filter
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "anull".into()),
        );
        let mut formats = if let Some(locked) = &self.output_format {
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
        filters.push(std::mem::take(&mut formats));
        if let Some(trim) = self.limits.trim(true, false) {
            filters.push(trim);
        }
        graph
            .output("in", 0)
            .and_then(|p| p.input("out", 0))
            .and_then(|p| p.parse(&filters.join(",")))
            .map_err(|e| util::error("创建音频滤镜图失败", e))?;
        graph
            .validate()
            .map_err(|e| util::error("配置音频滤镜图失败", e))?;
        if self.encoder.is_none() {
            let mut sink = graph.get("out").ok_or("音频滤镜输出不存在")?;
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
        self.graph = Some(graph);
        self.input_format = Some(input);
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
        match self.decoder.send_packet(packet) {
            Ok(()) => (),
            Err(e) if util::again(e) => {
                self.decode(output)?;
                self.decoder
                    .send_packet(packet)
                    .map_err(|e| util::error("提交音频包失败", e))?;
            }
            Err(e) => return Err(util::error("提交音频包失败", e)),
        }
        self.decode(output)?;
        Ok(())
    }
    fn decode(&mut self, packets: &mut EncodedPacketSink<'_>) -> Result<(), String> {
        let mut frame = frame::Audio::empty();
        while !self.done {
            util::check(&self.cancelled)?;
            match self.decoder.receive_frame(&mut frame) {
                Ok(()) => (),
                Err(ffmpeg::Error::Eof) => break,
                Err(e) if util::again(e) => break,
                Err(e) => return Err(util::error("解码音频帧失败", e)),
            }
            self.clock.normalize(&mut frame, self.base.into())?;
            let format = AudioFormat::from_frame(&frame)?;
            if self
                .input_format
                .as_ref()
                .is_some_and(|previous| !previous.matches(&format))
            {
                self.graph = None;
            }
            if self.graph.is_none() {
                self.configure(&frame)?;
            }
            unsafe {
                if (*frame.as_ptr()).ch_layout.order == ffi::AVChannelOrder::AV_CHANNEL_ORDER_UNSPEC
                {
                    ffi::av_channel_layout_uninit(&mut (*frame.as_mut_ptr()).ch_layout);
                    let result = ffi::av_channel_layout_copy(
                        &mut (*frame.as_mut_ptr()).ch_layout,
                        &self.input_format.as_ref().unwrap().layout,
                    );
                    if result < 0 {
                        return Err(util::error(
                            "复制音频输入声道失败",
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
                .ok_or("音频滤镜输入不存在")?;
            let result = unsafe {
                ffi::av_buffersrc_add_frame_flags(
                    source.as_mut_ptr(),
                    frame.as_mut_ptr(),
                    ffi::AV_BUFFERSRC_FLAG_KEEP_REF as i32,
                )
            };
            if result < 0 && result != ffi::AVERROR_EOF {
                return Err(util::error("提交音频滤镜失败", ffmpeg::Error::from(result)));
            }
            self.pull(packets)?;
        }
        Ok(())
    }
    fn pull(&mut self, packets: &mut EncodedPacketSink<'_>) -> Result<(), String> {
        loop {
            util::check(&self.cancelled)?;
            let mut frame = frame::Audio::empty();
            let mut sink = self
                .graph
                .as_mut()
                .unwrap()
                .get("out")
                .ok_or("音频滤镜输出不存在")?;
            match sink.sink().frame(&mut frame) {
                Ok(()) => (),
                Err(ffmpeg::Error::Eof) => {
                    self.done = true;
                    break;
                }
                Err(e) if util::again(e) => break,
                Err(e) => return Err(util::error("读取音频滤镜输出失败", e)),
            }
            if frame.samples() == 0 {
                continue;
            }
            let input_base: ffi::AVRational = sink.sink().time_base().into();
            let output_base: ffi::AVRational = self.encoder_base().into();
            let pts = frame.pts().ok_or("音频滤镜没有时间戳")?;
            frame.set_pts(Some(unsafe {
                ffi::av_rescale_q(pts, input_base, output_base)
                    - ffi::av_rescale_q(self.limits.output_start, util::MICROS, output_base)
            }));
            unsafe {
                (*frame.as_mut_ptr()).time_base = output_base;
                (*frame.as_mut_ptr()).duration = frame.samples() as i64;
            }
            if self.encoder.as_ref().unwrap().frame_size() == 0 {
                self.encode(&frame, packets)?;
            } else {
                self.queue.push(frame)?;
                self.queued(false, packets)?;
            }
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
        if !self.done {
            match self.decoder.send_eof() {
                Ok(()) | Err(ffmpeg::Error::Eof) => (),
                Err(e) if util::again(e) => {
                    self.decode(output)?;
                    self.decoder
                        .send_eof()
                        .map_err(|e| util::error("结束音频解码失败", e))?;
                }
                Err(e) => return Err(util::error("结束音频解码失败", e)),
            }
            self.decode(output)?;
            if !self.done {
                if let Some(graph) = self.graph.as_mut() {
                    graph
                        .get("in")
                        .ok_or("音频滤镜输入不存在")?
                        .source()
                        .flush()
                        .map_err(|e| util::error("结束音频滤镜失败", e))?;
                    self.pull(output)?;
                }
            }
        }
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

fn layout_name(layout: &ffi::AVChannelLayout) -> Result<String, String> {
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
