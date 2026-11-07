//! Demux-side DTS prediction used when a copied packet has no DTS. Matroska
//! B-frame streams commonly omit the initial decode timestamps; leaving those
//! to the muxer's guesses changes the resulting MP4 sample table.
use super::util;
use ffmpeg_next::{codec, ffi, format, Packet, Rational};

pub(super) struct PacketClock {
    next: Option<i64>,
    average_rate: Rational,
}

impl PacketClock {
    pub(super) fn new(stream: &format::stream::Stream<'_>) -> Self {
        Self {
            next: None,
            average_rate: stream.avg_frame_rate(),
        }
    }

    /// Return fftools' dts_est in microseconds without changing decoder input.
    pub(super) fn update(
        &mut self,
        packet: &Packet,
        base: Rational,
        parameters: &codec::Parameters,
        parser_repeat: Option<i32>,
    ) -> Result<i64, String> {
        let raw = unsafe { &*parameters.as_ptr() };
        let mut initial = 0;
        if self.next.is_none() {
            if self.average_rate.numerator() > 0 && self.average_rate.denominator() > 0 {
                initial = (-f64::from(raw.video_delay) * f64::from(ffi::AV_TIME_BASE)
                    / f64::from(self.average_rate)) as i64;
            }
            if let Some(pts) = packet.pts() {
                let pts = unsafe { ffi::av_rescale_q(pts, base.into(), util::MICROS) };
                initial = initial.checked_add(pts).ok_or("媒体预测DTS溢出")?;
            }
        }
        let current = packet
            .dts()
            .map(|dts| unsafe { ffi::av_rescale_q(dts, base.into(), util::MICROS) })
            .unwrap_or_else(|| self.next.unwrap_or(initial));
        let increment = match raw.codec_type {
            ffi::AVMediaType::AVMEDIA_TYPE_AUDIO if raw.sample_rate > 0 => {
                i64::from(ffi::AV_TIME_BASE) * i64::from(raw.frame_size)
                    / i64::from(raw.sample_rate)
            }
            ffi::AVMediaType::AVMEDIA_TYPE_AUDIO => unsafe {
                ffi::av_rescale_q(packet.duration(), base.into(), util::MICROS)
            },
            ffi::AVMediaType::AVMEDIA_TYPE_VIDEO if packet.duration() > 0 => unsafe {
                ffi::av_rescale_q(packet.duration(), base.into(), util::MICROS)
            },
            ffi::AVMediaType::AVMEDIA_TYPE_VIDEO
                if raw.framerate.num > 0 && raw.framerate.den > 0 =>
            {
                let descriptor = unsafe { ffi::avcodec_descriptor_get(raw.codec_id) };
                let fields = if !descriptor.is_null()
                    && unsafe { (*descriptor).props & ffi::AV_CODEC_PROP_FIELDS != 0 }
                {
                    parser_repeat.map_or(2, |repeat| 1 + repeat)
                } else {
                    2
                };
                let rate =
                    unsafe { ffi::av_mul_q(raw.framerate, ffi::AVRational { num: 2, den: 1 }) };
                unsafe {
                    ffi::av_rescale_q(
                        i64::from(fields),
                        ffi::AVRational {
                            num: rate.den,
                            den: rate.num,
                        },
                        util::MICROS,
                    )
                }
            }
            _ => 0,
        };
        self.next = Some(current.checked_add(increment).ok_or("媒体预测DTS溢出")?);
        Ok(current)
    }
}
