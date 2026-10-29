//! Demand-fed decoders for complex graphs. Only requested inputs are read;
//! pending frames from other streams of the same demuxer have a fixed bound.
use super::{audio_encoder::layout_name, util, video_clock::VideoClock, MediaInput};
use crate::audio::{AudioClock, AudioFormat};
use ffmpeg_next::{self as ffmpeg, codec, ffi, filter, format, frame, media, Packet, Rational};
use std::{collections::VecDeque, sync::atomic::AtomicBool};

pub(super) enum DecodedFrame {
    Audio(frame::Audio),
    Video(frame::Video),
}
impl DecodedFrame {
    pub(super) fn as_ptr(&self) -> *const ffi::AVFrame {
        unsafe {
            match self {
                Self::Audio(f) => f.as_ptr(),
                Self::Video(f) => f.as_ptr(),
            }
        }
    }
    pub(super) fn as_mut_ptr(&mut self) -> *mut ffi::AVFrame {
        unsafe {
            match self {
                Self::Audio(f) => f.as_mut_ptr(),
                Self::Video(f) => f.as_mut_ptr(),
            }
        }
    }
    pub(super) fn reference(&self) -> Result<Self, String> {
        let mut result = match self {
            Self::Audio(_) => Self::Audio(frame::Audio::empty()),
            Self::Video(_) => Self::Video(frame::Video::empty()),
        };
        let status = unsafe { ffi::av_frame_ref(result.as_mut_ptr(), self.as_ptr()) };
        if status < 0 {
            return Err(util::error(
                "引用滤镜输入帧失败",
                ffmpeg::Error::from(status),
            ));
        }
        Ok(result)
    }
    fn bytes(&self) -> usize {
        // Account native backing buffers rather than width*height heuristics:
        // an 8K YUV420 frame can still fit the bound, while float/planar formats
        // may use more than eight bytes per pixel.
        unsafe {
            let frame = &*self.as_ptr();
            let mut total = frame
                .buf
                .iter()
                .filter(|p| !p.is_null())
                .fold(0usize, |n, p| n.saturating_add((**p).size));
            for index in 0..frame.nb_extended_buf.max(0) as usize {
                let buffer = *frame.extended_buf.add(index);
                if !buffer.is_null() {
                    total = total.saturating_add((*buffer).size);
                }
            }
            total
        }
    }
    pub(super) fn matches(&self, other: &Self) -> Result<bool, String> {
        Ok(match (self, other) {
            (Self::Audio(a), Self::Audio(b)) => {
                AudioFormat::from_frame(a)?.matches(&AudioFormat::from_frame(b)?)
            }
            (Self::Video(a), Self::Video(b)) => unsafe {
                a.width() == b.width()
                    && a.height() == b.height()
                    && a.format() == b.format()
                    && a.aspect_ratio() == b.aspect_ratio()
                    && (*a.as_ptr()).colorspace == (*b.as_ptr()).colorspace
                    && (*a.as_ptr()).color_range == (*b.as_ptr()).color_range
                    && a.side_data(frame::side_data::Type::DisplayMatrix)
                        .and_then(|d| crate::video_frame::display_matrix(d.data()))
                        == b.side_data(frame::side_data::Type::DisplayMatrix)
                            .and_then(|d| crate::video_frame::display_matrix(d.data()))
            },
            _ => false,
        })
    }
    pub(super) fn buffer_args(&self, base: Rational, rate: Rational) -> Result<String, String> {
        match self {
            Self::Audio(f) => {
                let fmt = AudioFormat::from_frame(f)?;
                Ok(format!(
                    "time_base=1/{}:sample_rate={}:sample_fmt={}:channel_layout={}",
                    f.rate(),
                    f.rate(),
                    f.format().name(),
                    layout_name(&fmt.layout)?
                ))
            }
            Self::Video(f) => {
                let sar = if f.aspect_ratio().numerator() > 0 && f.aspect_ratio().denominator() > 0
                {
                    f.aspect_ratio()
                } else {
                    Rational(1, 1)
                };
                let mut args = format!(
                    "video_size={}x{}:pix_fmt={}:time_base={}/{}:pixel_aspect={}/{}",
                    f.width(),
                    f.height(),
                    unsafe { (*f.as_ptr()).format },
                    base.numerator(),
                    base.denominator(),
                    sar.numerator(),
                    sar.denominator()
                );
                if rate.numerator() > 0 && rate.denominator() > 0 {
                    args.push_str(&format!(
                        ":frame_rate={}/{}",
                        rate.numerator(),
                        rate.denominator()
                    ));
                }
                let buffer = filter::find("buffer").ok_or("缺少 buffer 滤镜")?;
                for (option, value) in [
                    (c"colorspace", unsafe { (*f.as_ptr()).colorspace as i32 }),
                    (c"range", unsafe { (*f.as_ptr()).color_range as i32 }),
                ] {
                    if crate::video_frame::filter_has_option(&buffer, option) {
                        args.push_str(&format!(
                            ":{}={value}",
                            option.to_str().map_err(|e| e.to_string())?
                        ));
                    }
                }
                #[cfg(bsr_ffmpeg_9_0)]
                if crate::video_frame::filter_has_option(&buffer, c"alpha_mode") {
                    args.push_str(&format!(":alpha_mode={}", unsafe {
                        (*f.as_ptr()).alpha_mode as i32
                    }));
                }
                Ok(args)
            }
        }
    }
}

enum Decoder {
    Audio(codec::decoder::Audio, AudioClock),
    Video(codec::decoder::Video, VideoClock),
}
pub(super) struct GraphDecoder {
    decoder: Decoder,
    pub(super) input: usize,
    pub(super) stream: usize,
    pub(super) base: Rational,
    pub(super) rate: Rational,
    pub(super) rotation: Option<[i32; 9]>,
    pub(super) template: Option<DecodedFrame>,
    pending: VecDeque<DecodedFrame>,
    pending_bytes: usize,
    packets: VecDeque<Packet>,
    packet_bytes: usize,
    input_ended: bool,
    pub(super) eof: bool,
}
impl GraphDecoder {
    pub(super) fn new(
        input: usize,
        stream: &format::stream::Stream<'_>,
        source: &MediaInput,
        context: &format::context::Input,
    ) -> Result<Self, String> {
        let base = stream.time_base();
        let parameters = stream.parameters();
        let decoder = util::open_decoder(parameters, base, source)?;
        let mut rate = Rational(0, 1);
        let mut rotation = None;
        let decoder = match stream.parameters().medium() {
            media::Type::Audio => Decoder::Audio(
                decoder
                    .audio()
                    .map_err(|e| util::error("音频图解码器错误", e))?,
                AudioClock::new(),
            ),
            media::Type::Video => {
                rate = unsafe {
                    ffi::av_guess_frame_rate(
                        context.as_ptr().cast_mut(),
                        stream.as_ptr().cast_mut(),
                        std::ptr::null_mut(),
                    )
                }
                .into();
                rotation = stream
                    .side_data()
                    .find(|s| s.kind() == ffmpeg::packet::side_data::Type::DisplayMatrix)
                    .and_then(|s| crate::video_frame::display_matrix(s.data()));
                let decoder = decoder
                    .video()
                    .map_err(|e| util::error("视频图解码器错误", e))?;
                let clock = VideoClock::new(
                    base,
                    stream.avg_frame_rate(),
                    decoder.frame_rate().unwrap_or(Rational(0, 1)),
                    unsafe { (*context.format().as_ptr()).flags & ffi::AVFMT_NOTIMESTAMPS != 0 },
                );
                Decoder::Video(decoder, clock)
            }
            _ => return Err("复杂图仅接受音频或视频输入".into()),
        };
        Ok(Self {
            decoder,
            input,
            stream: stream.index(),
            base,
            rate,
            rotation,
            template: None,
            pending: VecDeque::new(),
            pending_bytes: 0,
            packets: VecDeque::new(),
            packet_bytes: 0,
            input_ended: false,
            eof: false,
        })
    }
    pub(super) fn enqueue(&mut self, packet: &Packet) -> Result<(), String> {
        use ffmpeg::packet::{Mut, Ref};
        let bytes = self
            .packet_bytes
            .checked_add(packet.size())
            .ok_or("图输入压缩包队列溢出")?;
        if bytes > 64 * 1024 * 1024 || self.packets.len() >= 16_384 {
            return Err("复杂图同一输入的流读取速度不匹配，压缩包队列超出上限".into());
        }
        let mut copy = Packet::empty();
        let result = unsafe { ffi::av_packet_ref(copy.as_mut_ptr(), packet.as_ptr()) };
        if result < 0 {
            return Err(util::error(
                "引用图输入压缩包失败",
                ffmpeg::Error::from(result),
            ));
        }
        self.packet_bytes = bytes;
        self.packets.push_back(copy);
        Ok(())
    }
    pub(super) fn end_input(&mut self) {
        self.input_ended = true;
    }
    /// Decode only when this binding is requested. Other streams of the same
    /// demuxer remain compressed until their own graph request arrives.
    pub(super) fn advance(&mut self, cancelled: &AtomicBool) -> Result<bool, String> {
        if self.eof {
            return Ok(false);
        }
        if let Some(packet) = self.packets.pop_front() {
            self.packet_bytes -= packet.size();
            self.packet(Some(&packet), cancelled)?;
            return Ok(true);
        }
        if self.input_ended {
            self.packet(None, cancelled)?;
            return Ok(true);
        }
        Ok(false)
    }
    pub(super) fn packet(
        &mut self,
        packet: Option<&Packet>,
        cancelled: &AtomicBool,
    ) -> Result<(), String> {
        let result = match &mut self.decoder {
            Decoder::Audio(d, _) => match packet {
                Some(p) => d.send_packet(p),
                None => d.send_eof(),
            },
            Decoder::Video(d, _) => match packet {
                Some(p) => d.send_packet(p),
                None => d.send_eof(),
            },
        };
        match result {
            Ok(()) => (),
            Err(ffmpeg::Error::Eof) if packet.is_none() => {
                self.eof = true;
                return Ok(());
            }
            Err(e) if util::again(e) => {
                self.receive(cancelled)?;
                let retry = match &mut self.decoder {
                    Decoder::Audio(d, _) => match packet {
                        Some(p) => d.send_packet(p),
                        None => d.send_eof(),
                    },
                    Decoder::Video(d, _) => match packet {
                        Some(p) => d.send_packet(p),
                        None => d.send_eof(),
                    },
                };
                retry.map_err(|e| util::error("提交复杂图解码输入失败", e))?;
            }
            Err(e) => return Err(util::error("提交复杂图解码输入失败", e)),
        }
        self.receive(cancelled)?;
        if packet.is_none() && !self.eof {
            return Err("复杂图解码器结束后仍要求输入".into());
        }
        Ok(())
    }
    fn receive(&mut self, cancelled: &AtomicBool) -> Result<(), String> {
        loop {
            util::check(cancelled)?;
            let mut output = match self.decoder {
                Decoder::Audio(..) => DecodedFrame::Audio(frame::Audio::empty()),
                Decoder::Video(..) => DecodedFrame::Video(frame::Video::empty()),
            };
            let result = match (&mut self.decoder, &mut output) {
                (Decoder::Audio(d, _), DecodedFrame::Audio(f)) => d.receive_frame(f),
                (Decoder::Video(d, _), DecodedFrame::Video(f)) => d.receive_frame(f),
                _ => unreachable!(),
            };
            match result {
                Ok(()) => (),
                Err(ffmpeg::Error::Eof) => {
                    self.eof = true;
                    break;
                }
                Err(e) if util::again(e) => break,
                Err(e) => return Err(util::error("复杂图解码失败", e)),
            }
            match (&mut self.decoder, &mut output) {
                (Decoder::Audio(_, clock), DecodedFrame::Audio(f)) => {
                    clock.normalize(f, self.base.into())?;
                    let layout = AudioFormat::from_frame(f)?;
                    unsafe {
                        if (*f.as_ptr()).ch_layout.order
                            == ffi::AVChannelOrder::AV_CHANNEL_ORDER_UNSPEC
                        {
                            ffi::av_channel_layout_uninit(&mut (*f.as_mut_ptr()).ch_layout);
                            let code = ffi::av_channel_layout_copy(
                                &mut (*f.as_mut_ptr()).ch_layout,
                                &layout.layout,
                            );
                            if code < 0 {
                                return Err(util::error(
                                    "设置图输入声道失败",
                                    ffmpeg::Error::from(code),
                                ));
                            }
                        }
                    }
                }
                (Decoder::Video(d, clock), DecodedFrame::Video(f)) => {
                    clock.set_decoder_rate(d.frame_rate().unwrap_or(Rational(0, 1)));
                    clock.normalize(f)?;
                }
                _ => unreachable!(),
            }
            if self.template.is_none() {
                self.template = Some(output.reference()?);
            }
            let size = output.bytes();
            self.pending_bytes = self
                .pending_bytes
                .checked_add(size)
                .ok_or("图输入帧队列溢出")?;
            if self.pending_bytes > 64 * 1024 * 1024 || self.pending.len() >= 1024 {
                return Err("复杂图同一输入的流读取速度不匹配，帧队列超出上限".into());
            }
            self.pending.push_back(output);
        }
        Ok(())
    }
    /// Audio-only empty streams can still negotiate an encoder and produce
    /// container headers. Video needs a real decoded format frame.
    pub(super) fn empty_template(&self) -> Result<Option<DecodedFrame>, String> {
        let Decoder::Audio(decoder, _) = &self.decoder else {
            return Ok(None);
        };
        if decoder.rate() == 0 || decoder.format() == format::Sample::None {
            return Ok(None);
        }
        let mut frame = frame::Audio::empty();
        frame.set_rate(decoder.rate());
        frame.set_format(decoder.format());
        unsafe {
            let result = ffi::av_channel_layout_copy(
                &mut (*frame.as_mut_ptr()).ch_layout,
                &(*decoder.as_ptr()).ch_layout,
            );
            if result < 0 {
                return Err(util::error(
                    "复制空音轨声道失败",
                    ffmpeg::Error::from(result),
                ));
            }
            if (*frame.as_ptr()).ch_layout.nb_channels == 0 {
                return Ok(None);
            }
            (*frame.as_mut_ptr()).time_base = ffi::AVRational {
                num: 1,
                den: decoder.rate() as i32,
            };
        }
        Ok(Some(DecodedFrame::Audio(frame)))
    }
    pub(super) fn pop(&mut self) -> Option<DecodedFrame> {
        let f = self.pending.pop_front()?;
        self.pending_bytes -= f.bytes();
        Some(f)
    }
}
