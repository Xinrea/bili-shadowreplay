use super::{
    audio_encoder::{layout_name, AudioEncoderStage},
    util::{self, Limits},
    AudioEncodeOptions, EncodedPacketSink,
};
use crate::audio::{AudioClock, AudioFormat};
use ffmpeg_next::{self as ffmpeg, codec, ffi, filter, format, frame, Packet, Rational};
use std::sync::{atomic::AtomicBool, Arc};

pub(super) struct AudioStage {
    decoder: codec::decoder::Audio,
    encoding: AudioEncoderStage,
    graph: Option<filter::Graph>,
    input_format: Option<AudioFormat>,
    filter: Option<String>,
    base: Rational,
    limits: Limits,
    clock: AudioClock,
    done: bool,
    cancelled: Arc<AtomicBool>,
}

impl AudioStage {
    pub(super) fn new(
        parameters: codec::Parameters,
        source: &super::MediaInput,
        base: Rational,
        options: AudioEncodeOptions,
        limits: Limits,
        global_header: bool,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let filter = options.filter.clone();
        let encoding = AudioEncoderStage::new(options, limits, global_header, cancelled.clone())?;
        let decoder = util::open_decoder(parameters, base, source)?
            .audio()
            .map_err(|e| util::error("打开输入解码器失败", e))?;
        let mut output = Self {
            decoder,
            encoding,
            graph: None,
            input_format: None,
            filter,
            base,
            limits,
            clock: AudioClock::new(),
            done: false,
            cancelled,
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
        self.encoding.ready()
    }
    pub(super) fn done(&self) -> bool {
        self.done || self.encoding.done()
    }
    pub(super) fn parameters(&self) -> Option<codec::Parameters> {
        self.encoding.parameters()
    }
    pub(super) fn encoder_base(&self) -> Rational {
        self.encoding.encoder_base()
    }

    pub(super) fn samples(&self) -> u64 {
        self.encoding.samples()
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
            self.filter
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "anull".into()),
        );
        filters.push(self.encoding.format_filter()?);
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
        let mut sink = graph.get("out").ok_or("音频滤镜输出不存在")?;
        self.encoding.open_from_sink(&mut sink)?;
        self.graph = Some(graph);
        self.input_format = Some(input);
        Ok(())
    }

    pub(super) fn packet(
        &mut self,
        packet: &Packet,
        output: &mut EncodedPacketSink<'_>,
    ) -> Result<(), String> {
        if self.done || self.encoding.done() {
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
        while !self.done && !self.encoding.done() {
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
            let base = sink.sink().time_base();
            self.encoding.submit_filtered(frame, base, packets)?;
        }
        Ok(())
    }
    pub(super) fn finish(&mut self, output: &mut EncodedPacketSink<'_>) -> Result<(), String> {
        if !self.done && !self.encoding.done() {
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
            if !self.done && !self.encoding.done() {
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
        self.encoding.finish(output)?;
        self.done = true;
        Ok(())
    }
}
