//! Native subtitle decode/encode counterpart of fftools' do_subtitle_out.
//! Subtitle events bypass audio/video trim filters: output seek gates event
//! starts, output duration gates their unshifted event clock, and display spans
//! remain intact. Encoded packet PTS deliberately retain the event's input PTS.

use std::{
    ptr,
    sync::{atomic::AtomicBool, Arc},
};

use ffmpeg_next::{self as ffmpeg, codec, ffi, packet::Mut, Packet, Rational};

use super::{
    util::{self, Limits},
    EncodedPacketSink,
};

const MAX_PACKET: i32 = 1024 * 1024;

// ffmpeg-next's Subtitle wrapper has no Drop implementation. Every decode,
// including error/empty/flush paths, must release the native rectangles once.
struct Decoded(ffmpeg::Subtitle);
impl Decoded {
    fn new() -> Self {
        Self(ffmpeg::Subtitle::new())
    }
}
impl Drop for Decoded {
    fn drop(&mut self) {
        unsafe {
            ffi::avsubtitle_free(self.0.as_mut_ptr());
        }
    }
}

pub(super) struct SubtitleStage {
    decoder: codec::decoder::Subtitle,
    encoder: codec::encoder::subtitle::Encoder,
    limits: Limits,
    cancelled: Arc<AtomicBool>,
    scratch: Packet,
    done: bool,
}

impl SubtitleStage {
    pub(super) fn new(
        parameters: codec::Parameters,
        base: Rational,
        encoder: ffmpeg::Codec,
        limits: Limits,
        global_header: bool,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        util::check(&cancelled)?;
        if base.numerator() <= 0 || base.denominator() <= 0 {
            return Err("字幕输入时基无效".into());
        }
        if parameters.medium() != ffmpeg::media::Type::Subtitle
            || encoder.medium() != ffmpeg::media::Type::Subtitle
        {
            return Err("字幕阶段需要字幕解码器和编码器".into());
        }
        let mut decoder = codec::Context::from_parameters(parameters)
            .map_err(|error| util::error("创建字幕解码器失败", error))?
            .decoder();
        decoder.set_packet_time_base(base);
        let decoder = decoder
            .subtitle()
            .map_err(|error| util::error("打开字幕解码器失败", error))?;
        let mut context = codec::Context::new_with_codec(encoder)
            .encoder()
            .subtitle()
            .map_err(|error| util::error("创建字幕编码器失败", error))?;
        context.set_time_base(Rational(1, ffi::AV_TIME_BASE));
        unsafe {
            let source = &*decoder.as_ptr();
            let target = &mut *context.as_mut_ptr();
            target.width = source.width;
            target.height = source.height;
            if global_header {
                target.flags |= ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            }
            if !source.subtitle_header.is_null() {
                let size =
                    usize::try_from(source.subtitle_header_size).map_err(|_| "字幕头长度无效")?;
                // ASS encoders read a C string, but subtitle_header_size does
                // not include a terminator. The encoder context owns this copy.
                target.subtitle_header =
                    ffi::av_mallocz(size.checked_add(1).ok_or("字幕头长度溢出")?).cast();
                if target.subtitle_header.is_null() {
                    return Err("分配字幕头失败".into());
                }
                ptr::copy_nonoverlapping(source.subtitle_header, target.subtitle_header, size);
                target.subtitle_header_size = source.subtitle_header_size;
            }
        }
        let encoder = context
            .open_as(encoder)
            .map_err(|error| util::error("打开字幕编码器失败", error))?;
        Ok(Self {
            decoder,
            encoder,
            limits,
            cancelled,
            scratch: Packet::empty(),
            done: false,
        })
    }

    pub(super) fn parameters(&self) -> codec::Parameters {
        codec::Parameters::from(&self.encoder)
    }
    pub(super) fn time_base(&self) -> Rational {
        Rational(1, ffi::AV_TIME_BASE)
    }
    pub(super) fn done(&self) -> bool {
        self.done
    }

    pub(super) fn packet(
        &mut self,
        packet: &Packet,
        output: &mut EncodedPacketSink<'_>,
    ) -> Result<(), String> {
        util::check(&self.cancelled)?;
        if self.done {
            return Ok(());
        }
        let mut subtitle = Decoded::new();
        let got = self
            .decoder
            .decode(packet, &mut subtitle.0)
            .map_err(|error| util::error("解码字幕失败", error))?;
        if got {
            self.encode(&subtitle.0, output)
        } else {
            Ok(())
        }
    }

    pub(super) fn finish(&mut self, output: &mut EncodedPacketSink<'_>) -> Result<(), String> {
        util::check(&self.cancelled)?;
        while !self.done {
            util::check(&self.cancelled)?;
            let mut subtitle = Decoded::new();
            let got = self
                .decoder
                .decode(&Packet::empty(), &mut subtitle.0)
                .map_err(|error| util::error("结束字幕解码失败", error))?;
            if !got {
                self.done = true;
                break;
            }
            self.encode(&subtitle.0, output)?;
        }
        Ok(())
    }

    fn encode(
        &mut self,
        subtitle: &ffmpeg::Subtitle,
        output: &mut EncodedPacketSink<'_>,
    ) -> Result<(), String> {
        util::check(&self.cancelled)?;
        // CLI's non--xerror path drops events without an event timestamp.
        let Some(pts) = subtitle.pts() else {
            return Ok(());
        };
        if self.limits.output_seek && pts < self.limits.output_start {
            return Ok(());
        }
        let local_pts = pts
            .checked_sub(self.limits.output_start)
            .ok_or("字幕时间戳溢出")?;
        if !self.limits.encode(local_pts, util::MICROS) {
            self.done = true;
            return Ok(());
        }
        // Input -t only limits stream-copy demuxing and audio/video trim
        // filters in fftools. It does not truncate encoded subtitle events.
        let raw = unsafe { &*subtitle.as_ptr() };
        let codec = self.encoder.id();
        let count = if codec == codec::Id::DVB_SUBTITLE {
            2
        } else if codec == codec::Id::ASS {
            raw.num_rects.max(1)
        } else {
            1
        };
        if self.scratch.size() == 0 {
            let result = unsafe { ffi::av_new_packet(self.scratch.as_mut_ptr(), MAX_PACKET) };
            if result < 0 {
                return Err(util::error(
                    "分配字幕编码缓冲区失败",
                    ffmpeg::Error::from(result),
                ));
            }
        }
        for index in 0..count {
            util::check(&self.cancelled)?;
            // Borrow the original rectangles. Only Decoded owns/frees them.
            let mut local = unsafe { ptr::read(raw) };
            local.pts = local_pts
                .checked_add(i64::from(raw.start_display_time) * 1000)
                .ok_or("字幕显示时间戳溢出")?;
            local.end_display_time = raw
                .end_display_time
                .checked_sub(raw.start_display_time)
                .ok_or("字幕结束时间早于开始时间")?;
            local.start_display_time = 0;
            if codec == codec::Id::DVB_SUBTITLE && index == 1 {
                local.num_rects = 0;
            } else if codec == codec::Id::ASS && raw.num_rects > 0 {
                if local.rects.is_null() {
                    return Err("字幕缺少矩形数据".into());
                }
                local.num_rects = 1;
                local.rects = unsafe { local.rects.add(index as usize) };
            }
            let size = unsafe {
                ffi::avcodec_encode_subtitle(
                    self.encoder.as_mut_ptr(),
                    (*self.scratch.as_mut_ptr()).data,
                    MAX_PACKET,
                    &local,
                )
            };
            if size < 0 {
                return Err(util::error("编码字幕失败", ffmpeg::Error::from(size)));
            }
            if size > MAX_PACKET {
                return Err("字幕编码包超过缓冲区".into());
            }
            util::check(&self.cancelled)?;
            // Shrinking a 1 MiB AVPacket only changes its logical size. Copy
            // into an exact-sized buffer before a muxer may queue it, so its
            // bounded pending-byte accounting reflects the retained storage.
            let mut packet = Packet::empty();
            let allocated = unsafe { ffi::av_new_packet(packet.as_mut_ptr(), size) };
            if allocated < 0 {
                return Err(util::error(
                    "分配字幕输出包失败",
                    ffmpeg::Error::from(allocated),
                ));
            }
            if size > 0 {
                unsafe {
                    ptr::copy_nonoverlapping(
                        (*self.scratch.as_mut_ptr()).data,
                        (*packet.as_mut_ptr()).data,
                        size as usize,
                    );
                }
            }
            let packet_pts = if codec == codec::Id::DVB_SUBTITLE {
                pts.checked_add(
                    i64::from(if index == 0 {
                        raw.start_display_time
                    } else {
                        raw.end_display_time
                    }) * 1000,
                )
                .ok_or("字幕输出时间戳溢出")?
            } else {
                pts
            };
            // This is intentionally original PTS, not local_pts: it matches
            // both FFmpeg 6/9 do_subtitle_out, including output-side -ss.
            packet.set_pts(Some(packet_pts));
            packet.set_dts(Some(packet_pts));
            packet.set_duration(i64::from(raw.end_display_time) * 1000);
            unsafe {
                (*packet.as_mut_ptr()).time_base = util::MICROS;
            }
            output(packet, self.time_base())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
