//! FFmpeg CLI-compatible decoder clocks and output video frame scheduling.
//! Checked against n6.1.3/n9.0.2 fftools/ffmpeg_dec.c video_frame_process and
//! ffmpeg_filter.c video_sync_process/adjust_frame_pts_to_encoder_tb. These
//! include fractional-tick rounding, previous-frame duplication and EOF tails;
//! a plain fps filter does not implement the same scheduling contract.

use ffmpeg_next::{self as ffmpeg, ffi, frame, Rational};

use super::FrameRatePolicy;

fn valid(value: Rational) -> bool {
    value.numerator() > 0 && value.denominator() > 0
}

pub(super) struct VideoClock {
    packet_base: Rational,
    average_rate: Rational,
    decoder_rate: Rational,
    unreliable: bool,
    last_pts: Option<i64>,
    last_duration: i64,
}

impl VideoClock {
    pub(super) fn new(
        packet_base: Rational,
        average_rate: Rational,
        decoder_rate: Rational,
        timestamps_unreliable: bool,
    ) -> Self {
        Self {
            packet_base,
            average_rate,
            decoder_rate,
            unreliable: timestamps_unreliable,
            last_pts: None,
            last_duration: 0,
        }
    }

    pub(super) fn set_decoder_rate(&mut self, rate: Rational) {
        self.decoder_rate = rate;
    }

    pub(super) fn normalize(&mut self, frame: &mut frame::Video) -> Result<(), String> {
        if !valid(self.packet_base) {
            return Err("视频解码时基无效".into());
        }
        let pts = match frame.timestamp() {
            Some(pts) => pts,
            None => match self.last_pts {
                Some(previous) => previous
                    .checked_add(self.last_duration)
                    .ok_or("视频解码时间戳溢出")?,
                None => 0,
            },
        };
        frame.set_pts(Some(pts));
        unsafe {
            (*frame.as_mut_ptr()).time_base = self.packet_base.into();
        }
        let duration = unsafe { (*frame.as_ptr()).duration };
        let difference = self
            .last_pts
            .and_then(|previous| pts.checked_sub(previous))
            .filter(|delta| *delta > 0);
        // Newer ffmpeg disregards a synthetic one-tick duration when real
        // timestamps demonstrate that it is too short. 6.1 does not.
        let synthetic = ffmpeg::codec::version() >> 16 >= 63
            && duration == 1
            && difference.is_some_and(|delta| delta > 2);
        let codec_duration = if valid(self.decoder_rate) {
            let fields = unsafe { (*frame.as_ptr()).repeat_pict }
                .checked_add(2)
                .ok_or("视频重复场数量溢出")?;
            unsafe {
                ffi::av_rescale_q(
                    i64::from(fields),
                    ffi::av_inv_q(ffi::av_mul_q(
                        self.decoder_rate.into(),
                        ffi::AVRational { num: 2, den: 1 },
                    )),
                    self.packet_base.into(),
                )
            }
        } else {
            0
        };
        let average_duration = if valid(self.average_rate) {
            unsafe {
                ffi::av_rescale_q(
                    1,
                    ffi::av_inv_q(self.average_rate.into()),
                    self.packet_base.into(),
                )
            }
        } else {
            0
        };
        let estimated = if duration > 0 && !self.unreliable && !synthetic {
            duration
        } else if codec_duration > 0 && self.unreliable {
            codec_duration
        } else if let Some(difference) = difference {
            difference
        } else if duration > 0 {
            duration
        } else if codec_duration > 0 {
            codec_duration
        } else if average_duration > 0 {
            average_duration
        } else {
            self.last_duration.max(1)
        };
        self.last_pts = Some(pts);
        self.last_duration = estimated;
        // Estimated duration is only a fallback for a later frame with no PTS.
        // The filtered frame retains its actual decoder-provided duration.
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Constant,
    ConstantOffset,
    Variable,
    Passthrough,
}

pub(super) struct FrameSync {
    policy: FrameRatePolicy,
    filter_base: Rational,
    encoder_base: Rational,
    mux_flags: i32,
    output_start: i64,
    initial_gap: bool,
    avi: bool,
    next_pts: i64,
    emitted: i64,
    previous_counts: [i64; 3],
    last_dropped: bool,
    dropped_keyframe: bool,
    previous: Option<frame::Video>,
    finished: bool,
}

impl FrameSync {
    pub(super) fn new(
        policy: FrameRatePolicy,
        _frame_rate: Rational,
        filter_base: Rational,
        encoder_base: Rational,
        mux_flags: i32,
        output_start_us: i64,
    ) -> Result<Self, String> {
        if !valid(filter_base) || !valid(encoder_base) {
            return Err("视频输出时基无效".into());
        }
        if let FrameRatePolicy::Constant { num, den } = policy {
            if num == 0 || den == 0 || num > i32::MAX as u32 || den > i32::MAX as u32 {
                return Err("视频输出帧率无效".into());
            }
        }
        Ok(Self {
            policy,
            filter_base,
            encoder_base,
            mux_flags,
            output_start: output_start_us,
            initial_gap: false,
            avi: false,
            next_pts: 0,
            emitted: 0,
            previous_counts: [0; 3],
            last_dropped: false,
            dropped_keyframe: false,
            previous: None,
            finished: false,
        })
    }

    /// Match CLI's VSCFR exception for a single input stream without -itsoffset,
    /// or -copyts. Explicit output -r still uses this exception under fps_mode auto.
    pub(super) fn preserve_initial_gap(mut self, preserve: bool) -> Self {
        self.initial_gap = preserve;
        self
    }
    /// AVI's fps_mode auto is VFR even though its muxer flags do not imply it.
    pub(super) fn avi_auto(mut self, avi: bool) -> Self {
        self.avi = avi;
        self
    }

    fn mode(&self) -> Mode {
        let mode = match self.policy {
            FrameRatePolicy::Passthrough => Mode::Passthrough,
            FrameRatePolicy::Variable => Mode::Variable,
            FrameRatePolicy::Constant { .. } => Mode::Constant,
            FrameRatePolicy::Auto if self.avi => Mode::Variable,
            FrameRatePolicy::Auto if self.mux_flags & ffi::AVFMT_VARIABLE_FPS != 0 => {
                if self.mux_flags & ffi::AVFMT_NOTIMESTAMPS != 0 {
                    Mode::Passthrough
                } else {
                    Mode::Variable
                }
            }
            FrameRatePolicy::Auto => Mode::Constant,
        };
        if mode == Mode::Constant && self.initial_gap {
            Mode::ConstantOffset
        } else {
            mode
        }
    }

    pub(super) fn push(&mut self, frame: Option<&frame::Video>) -> Result<FrameBatch, String> {
        if self.finished {
            if frame.is_some() {
                return Err("视频帧同步器已结束".into());
            }
            return Ok(FrameBatch::empty(self.encoder_base));
        }
        let mode = self.mode();
        let mut current = frame.map(reference).transpose()?;
        let (mut count, previous_count) = if let Some(frame) = current.as_mut() {
            let source_base = unsafe { Rational::from((*frame.as_ptr()).time_base) };
            let source_base = if valid(source_base) {
                source_base
            } else {
                self.filter_base
            };
            let mut duration = unsafe { (*frame.as_ptr()).duration } as f64
                * f64::from(source_base)
                / f64::from(self.encoder_base);
            let mut position =
                adjusted_position(frame, source_base, self.encoder_base, self.output_start)?;
            let mut drift = position - self.next_pts as f64;
            let mut span = drift + duration;
            let mut previous = 0;
            let mut count = 1;
            if drift < 0.0 && span > 0.0 && mode != Mode::Passthrough {
                position = self.next_pts as f64;
                duration += drift;
                drift = 0.0;
            }
            if mode == Mode::ConstantOffset && self.emitted == 0 && drift >= 0.5 {
                span = duration;
                drift = 0.0;
                self.next_pts = rounded(position)?;
            }
            match mode {
                Mode::Constant | Mode::ConstantOffset => {
                    if span < -1.1 {
                        count = 0;
                    } else if span > 1.1 {
                        count = rounded_single(span)?;
                        if drift > 1.1 {
                            previous = rounded_single(drift - 0.6)?;
                        }
                    }
                    unsafe {
                        (*frame.as_mut_ptr()).duration = 1;
                    }
                }
                Mode::Variable => {
                    if span <= -0.6 {
                        count = 0;
                    } else if span > 0.6 {
                        self.next_pts = rounded(position)?;
                    }
                    unsafe {
                        (*frame.as_mut_ptr()).duration = rounded(duration)?;
                    }
                }
                Mode::Passthrough => {
                    self.next_pts = rounded(position)?;
                    unsafe {
                        (*frame.as_mut_ptr()).duration = rounded(duration)?;
                    }
                }
            }
            (count, previous)
        } else {
            let mut sorted = self.previous_counts;
            sorted.sort_unstable();
            self.finished = true;
            (sorted[1], sorted[1])
        };
        self.previous_counts.rotate_right(1);
        self.previous_counts[0] = previous_count;
        // Same maximum as the CLI's default dts_error_threshold (30 hours) ×30.
        // Generate acceptable batches lazily, so even a long gap does not
        // preallocate millions of frame headers or copy their pixel buffers.
        let duplicate = count
            > i64::from(previous_count > 0 && self.last_dropped)
                + i64::from(count > previous_count);
        let excessive = duplicate && count > 3_240_000;
        if excessive {
            count = 0;
        } else {
            self.last_dropped = count == previous_count && current.is_some();
            if self.last_dropped && current.as_ref().is_some_and(is_key) {
                self.dropped_keyframe = true;
            }
        }
        let previous = self.previous.as_ref().map(reference).transpose()?;
        if current.is_none() && previous.is_none() {
            count = 0;
        }
        let first_pts = self.next_pts;
        self.next_pts = self
            .next_pts
            .checked_add(count)
            .ok_or("视频输出时间戳溢出")?;
        self.emitted = self.emitted.checked_add(count).ok_or("视频输出帧数溢出")?;
        let force_key = count > 0 && self.dropped_keyframe;
        if count > 0 {
            self.dropped_keyframe = false;
        }
        if let Some(frame) = current.as_ref() {
            let mut stored = reference(frame)?;
            if count > previous_count {
                unsafe {
                    (*stored.as_mut_ptr()).flags &= !(ffi::AV_FRAME_FLAG_KEY);
                }
            }
            self.previous = Some(stored);
        }
        Ok(FrameBatch {
            current,
            previous,
            count,
            previous_count,
            emitted: 0,
            first_pts,
            encoder_base: self.encoder_base,
            force_key,
        })
    }
}

fn is_key(frame: &frame::Video) -> bool {
    unsafe { (*frame.as_ptr()).flags & ffi::AV_FRAME_FLAG_KEY != 0 }
}

fn reference(source: &frame::Video) -> Result<frame::Video, String> {
    let mut target = frame::Video::empty();
    let result = unsafe { ffi::av_frame_ref(target.as_mut_ptr(), source.as_ptr()) };
    if result < 0 {
        Err(format!("引用视频帧失败: {}", ffmpeg::Error::from(result)))
    } else {
        Ok(target)
    }
}

fn rounded(value: f64) -> Result<i64, String> {
    if !value.is_finite()
        || !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&value)
    {
        return Err("视频帧同步时间戳超出范围".into());
    }
    // C llrint/llrintf use the current nearest-even rounding mode. Rust's
    // round() rounds midpoint away from zero and changes dup/drop boundaries.
    Ok(unsafe { ffi::llrint(value) })
}

fn rounded_single(value: f64) -> Result<i64, String> {
    let value = value as f32;
    rounded(f64::from(value))?;
    Ok(unsafe { ffi::llrintf(value) })
}

fn adjusted_position(
    frame: &mut frame::Video,
    source_base: Rational,
    encoder_base: Rational,
    start: i64,
) -> Result<f64, String> {
    let pts = frame.pts().ok_or("滤镜输出视频帧没有时间戳")?;
    let mut precise: ffi::AVRational = encoder_base.into();
    let extra = (29 - unsafe { ffi::av_log2(precise.den as u32) }).clamp(0, 16);
    precise.den <<= extra;
    let micros = ffi::AVRational {
        num: 1,
        den: ffi::AV_TIME_BASE,
    };
    let exact = unsafe { ffi::av_rescale_q(pts, source_base.into(), precise) }
        .checked_sub(unsafe { ffi::av_rescale_q(start, micros, precise) })
        .ok_or("视频输出时间戳溢出")?;
    let mut position = exact as f64 / f64::from(1 << extra);
    if position != rounded(position)? as f64 {
        position += position.signum() / f64::from(1 << 17);
    }
    let pts = unsafe { ffi::av_rescale_q(pts, source_base.into(), encoder_base.into()) }
        .checked_sub(unsafe { ffi::av_rescale_q(start, micros, encoder_base.into()) })
        .ok_or("视频输出时间戳溢出")?;
    frame.set_pts(Some(pts));
    unsafe {
        (*frame.as_mut_ptr()).time_base = encoder_base.into();
    }
    Ok(position)
}

/// Reference-counted, lazy scheduled frames. Consume a batch before pushing
/// another source frame, unless the entire output is stopping at its limit.
pub(super) struct FrameBatch {
    current: Option<frame::Video>,
    previous: Option<frame::Video>,
    count: i64,
    previous_count: i64,
    emitted: i64,
    first_pts: i64,
    encoder_base: Rational,
    force_key: bool,
}

impl FrameBatch {
    fn empty(encoder_base: Rational) -> Self {
        Self {
            current: None,
            previous: None,
            count: 0,
            previous_count: 0,
            emitted: 0,
            first_pts: 0,
            encoder_base,
            force_key: false,
        }
    }
}

impl Iterator for FrameBatch {
    type Item = Result<frame::Video, String>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.emitted >= self.count {
            return None;
        }
        let index = self.emitted;
        let source = if index < self.previous_count && self.previous.is_some() {
            self.previous.as_ref()
        } else {
            self.current.as_ref()
        }?;
        self.emitted += 1;
        Some(reference(source).map(|mut frame| {
            frame.set_pts(Some(self.first_pts + index));
            unsafe {
                (*frame.as_mut_ptr()).time_base = self.encoder_base.into();
                if index > self.previous_count {
                    (*frame.as_mut_ptr()).flags &= !(ffi::AV_FRAME_FLAG_KEY);
                }
                if self.force_key && index == 0 {
                    (*frame.as_mut_ptr()).flags |= ffi::AV_FRAME_FLAG_KEY;
                }
            }
            frame
        }))
    }
}

#[cfg(test)]
mod tests;
