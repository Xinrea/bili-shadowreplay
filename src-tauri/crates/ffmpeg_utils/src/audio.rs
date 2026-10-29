//! Streaming mono PCM decoding. The sink and its output guards stay on the
//! blocking worker until native I/O and decoder work have stopped.

use std::{
    ops::ControlFlow,
    path::Path,
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use ffmpeg_next::{self as ffmpeg, codec, ffi, format, frame, media, Packet};

use crate::metadata::{open_input, run_media};

mod range;
pub use range::{decode_audio_range, AudioRange};
mod opus;
pub use opus::{encode_opus_sample, encode_opus_upload};
mod encode;
mod queue;
pub(crate) use encode::{copy_chapters, input_metadata};
pub(crate) use queue::AudioFrameQueue;
pub(crate) use range::AudioClock;
mod import;
pub(crate) mod remux;
pub use encode::encode_mp3;
pub use import::{import_audio, ImportAudioMode};
mod flac_preview;
pub use flac_preview::{encode_flac_preview, AudioTiming};

pub trait AudioSink: Send + 'static {
    fn write(&mut self, samples: &[i16]) -> Result<(), String>;

    fn finish(&mut self) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AudioDecodeOptions {
    pub sample_rate: u32,
}

pub struct AudioDecodeResult<S> {
    pub sink: S,
    pub samples: u64,
    pub has_audio: bool,
}

/// Decode the CLI's automatically selected audio stream to signed 16-bit mono.
pub async fn decode_audio<S: AudioSink>(
    path: &Path,
    options: AudioDecodeOptions,
    sink: S,
) -> Result<AudioDecodeResult<S>, String> {
    if !(1..=384_000).contains(&options.sample_rate) {
        return Err("音频采样率必须在 1 到 384000 之间".into());
    }
    let path = path.to_path_buf();
    run_media(move |cancelled| decode_sync(&path, options, sink, cancelled)).await
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Acquire) {
        Err("音频解码已取消".into())
    } else {
        Ok(())
    }
}

fn native_error(context: &str, error: impl std::fmt::Display) -> String {
    format!("{context}: {error}")
}

fn again(error: ffmpeg::Error) -> bool {
    matches!(error, ffmpeg::Error::Other { errno } if errno == ffi::EAGAIN)
}

/// Match fftools' auto-audio mapping, including its preference for default
/// tracks. av_find_best_stream has different tie breakers (e.g. bit rate).
pub(crate) fn audio_stream(input: &format::context::Input) -> Option<usize> {
    let mut selected = None;
    let mut highest = i64::MIN;
    for stream in input.streams() {
        if stream.parameters().medium() != media::Type::Audio {
            continue;
        }
        // SAFETY: these stream and parameter pointers are owned by input.
        let score = unsafe {
            let stream = &*stream.as_ptr();
            if stream.discard == ffi::AVDiscard::AVDISCARD_ALL {
                continue;
            }
            i64::from((*stream.codecpar).ch_layout.nb_channels)
                + if stream.event_flags & ffi::AVSTREAM_EVENT_FLAG_NEW_PACKETS != 0 {
                    100_000_000
                } else {
                    0
                }
                + if stream.disposition & ffi::AV_DISPOSITION_DEFAULT != 0 {
                    5_000_000
                } else {
                    0
                }
        };
        if score > highest {
            highest = score;
            selected = Some(stream.index());
        }
    }
    selected
}

fn decode_sync<S: AudioSink>(
    path: &Path,
    options: AudioDecodeOptions,
    mut sink: S,
    cancelled: Arc<AtomicBool>,
) -> Result<AudioDecodeResult<S>, String> {
    let mut input = open_input(path, cancelled.clone())?;
    let Some(index) = audio_stream(&input) else {
        check_cancelled(&cancelled)?;
        sink.finish()?;
        return Ok(AudioDecodeResult {
            sink,
            samples: 0,
            has_audio: false,
        });
    };
    let stream = input.stream(index).ok_or("找不到音轨")?;
    let time_base = stream.time_base();
    let context = codec::context::Context::from_parameters(stream.parameters())
        .map_err(|error| native_error("初始化音频解码器失败", error))?;
    let mut decoder = context.decoder();
    decoder.set_packet_time_base(time_base);
    let mut decoder = decoder
        .audio()
        .map_err(|error| native_error("打开音频解码器失败", error))?;
    let mut pipeline = Pipeline::new(options.sample_rate, sink, cancelled.clone());

    decode_frames(
        &mut input,
        index,
        &mut decoder,
        &cancelled,
        |_| Ok(()),
        |frame| {
            pipeline.frame(frame)?;
            Ok(ControlFlow::Continue(()))
        },
    )?;
    pipeline.finish()
}

fn decode_frames<P, F>(
    input: &mut format::context::Input,
    index: usize,
    decoder: &mut codec::decoder::Audio,
    cancelled: &AtomicBool,
    mut packet_map: P,
    mut consume: F,
) -> Result<(), String>
where
    P: FnMut(&mut Packet) -> Result<(), String>,
    F: FnMut(&mut frame::Audio) -> Result<ControlFlow<()>, String>,
{
    let mut decoded = frame::Audio::empty();
    loop {
        check_cancelled(cancelled)?;
        // av_read_frame needs an empty packet. Drop every reference, including
        // skipped video packets, rather than retaining the recording's payload.
        let mut packet = Packet::empty();
        match packet.read(input) {
            Ok(()) => {
                if packet.stream() != index {
                    continue;
                }
                packet_map(&mut packet)?;
                match decoder.send_packet(&packet) {
                    Ok(()) => (),
                    Err(error) if again(error) => {
                        if drain_decoder(decoder, &mut decoded, cancelled, &mut consume)?.is_break()
                        {
                            return Ok(());
                        }
                        decoder
                            .send_packet(&packet)
                            .map_err(|error| native_error("提交音频包失败", error))?;
                    }
                    Err(error) => return Err(native_error("提交音频包失败", error)),
                }
                if drain_decoder(decoder, &mut decoded, cancelled, &mut consume)?.is_break() {
                    return Ok(());
                }
            }
            Err(ffmpeg::Error::Eof) => break,
            Err(error) => return Err(native_error("读取音频包失败", error)),
        }
    }
    match decoder.send_eof() {
        Ok(()) | Err(ffmpeg::Error::Eof) => (),
        Err(error) if again(error) => {
            if drain_decoder(decoder, &mut decoded, cancelled, &mut consume)?.is_break() {
                return Ok(());
            }
            decoder
                .send_eof()
                .map_err(|error| native_error("结束音频解码失败", error))?;
        }
        Err(error) => return Err(native_error("结束音频解码失败", error)),
    }
    let _ = drain_decoder(decoder, &mut decoded, cancelled, &mut consume)?;
    Ok(())
}

fn drain_decoder<F>(
    decoder: &mut codec::decoder::Audio,
    frame: &mut frame::Audio,
    cancelled: &AtomicBool,
    consume: &mut F,
) -> Result<ControlFlow<()>, String>
where
    F: FnMut(&mut frame::Audio) -> Result<ControlFlow<()>, String>,
{
    loop {
        check_cancelled(cancelled)?;
        match decoder.receive_frame(frame) {
            Ok(()) => {
                if consume(frame)?.is_break() {
                    return Ok(ControlFlow::Break(()));
                }
            }
            Err(ffmpeg::Error::Eof) => break,
            Err(error) if again(error) => break,
            Err(error) => return Err(native_error("解码音频失败", error)),
        }
    }
    Ok(ControlFlow::Continue(()))
}

pub(crate) struct AudioFormat {
    pub(crate) layout: ffi::AVChannelLayout,
    pub(crate) sample_format: ffi::AVSampleFormat,
    pub(crate) sample_rate: i32,
}

impl AudioFormat {
    pub(crate) fn from_frame(frame: &frame::Audio) -> Result<Self, String> {
        let mut layout = unsafe { std::mem::zeroed::<ffi::AVChannelLayout>() };
        // SAFETY: decoded owns valid codec-provided channel metadata. Own a
        // copy, including custom channel maps, beyond this frame's lifetime.
        unsafe {
            let input = &(*frame.as_ptr()).ch_layout;
            if input.nb_channels <= 0 || frame.rate() == 0 || frame.rate() > i32::MAX as u32 {
                return Err("音频帧缺少采样率或声道信息".into());
            }
            if input.order == ffi::AVChannelOrder::AV_CHANNEL_ORDER_UNSPEC {
                ffi::av_channel_layout_default(&mut layout, input.nb_channels);
            } else {
                let result = ffi::av_channel_layout_copy(&mut layout, input);
                if result < 0 {
                    return Err(native_error(
                        "复制音频声道布局失败",
                        ffmpeg::Error::from(result),
                    ));
                }
            }
        }
        Ok(Self {
            layout,
            sample_format: frame.format().into(),
            sample_rate: frame.rate() as i32,
        })
    }

    pub(crate) fn matches(&self, other: &Self) -> bool {
        self.sample_format == other.sample_format
            && self.sample_rate == other.sample_rate
            && unsafe { ffi::av_channel_layout_compare(&self.layout, &other.layout) == 0 }
    }
}

impl Drop for AudioFormat {
    fn drop(&mut self) {
        unsafe { ffi::av_channel_layout_uninit(&mut self.layout) };
    }
}

struct Resampler {
    ptr: *mut ffi::SwrContext,
    format: AudioFormat,
}

impl Resampler {
    fn new(format: AudioFormat, rate: u32) -> Result<Self, String> {
        let mut result = Self {
            ptr: ptr::null_mut(),
            format,
        };
        let mut output = unsafe { std::mem::zeroed::<ffi::AVChannelLayout>() };
        // SAFETY: swr owns copies of both layouts; output is native mono.
        unsafe {
            ffi::av_channel_layout_default(&mut output, 1);
            let ret = ffi::swr_alloc_set_opts2(
                &mut result.ptr,
                &output,
                ffi::AVSampleFormat::AV_SAMPLE_FMT_S16,
                rate as i32,
                &result.format.layout,
                result.format.sample_format,
                result.format.sample_rate,
                0,
                ptr::null_mut(),
            );
            ffi::av_channel_layout_uninit(&mut output);
            if ret < 0 {
                return Err(native_error(
                    "创建音频重采样器失败",
                    ffmpeg::Error::from(ret),
                ));
            }
            let ret = ffi::swr_init(result.ptr);
            if ret < 0 {
                return Err(native_error(
                    "初始化音频重采样器失败",
                    ffmpeg::Error::from(ret),
                ));
            }
        }
        Ok(result)
    }

    fn convert(&mut self, input: *const *const u8, count: i32) -> Result<Vec<i16>, String> {
        // SAFETY: swr reports a bound that includes buffered samples. Reserving
        // this capacity avoids accumulating an upsampled recording internally.
        let capacity = unsafe { ffi::swr_get_out_samples(self.ptr, count) };
        if capacity < 0 {
            return Err(native_error(
                "计算重采样容量失败",
                ffmpeg::Error::from(capacity),
            ));
        }
        let capacity = capacity.max(1) as usize;
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(capacity)
            .map_err(|error| native_error("分配音频缓冲失败", error))?;
        samples.resize(capacity, 0);
        let mut data = [samples.as_mut_ptr().cast::<u8>()];
        // SAFETY: output has `capacity` mono i16 slots; input planes come from
        // the live decoded frame, or are null when draining.
        let written = unsafe {
            // FFmpeg 6 uses uint8_t const **; newer headers additionally make
            // the plane-pointer array const. A mutable array coerces to both.
            ffi::swr_convert(
                self.ptr,
                data.as_mut_ptr(),
                capacity as i32,
                input.cast_mut(),
                count,
            )
        };
        if written < 0 {
            return Err(native_error("音频重采样失败", ffmpeg::Error::from(written)));
        }
        samples.truncate(written as usize);
        Ok(samples)
    }
}

impl Drop for Resampler {
    fn drop(&mut self) {
        unsafe { ffi::swr_free(&mut self.ptr) };
    }
}

struct Pipeline<S> {
    sink: S,
    rate: u32,
    samples: u64,
    resampler: Option<Resampler>,
    cancelled: Arc<AtomicBool>,
}

impl<S: AudioSink> Pipeline<S> {
    fn new(rate: u32, sink: S, cancelled: Arc<AtomicBool>) -> Self {
        Self {
            sink,
            rate,
            samples: 0,
            resampler: None,
            cancelled,
        }
    }

    fn frame(&mut self, frame: &frame::Audio) -> Result<(), String> {
        check_cancelled(&self.cancelled)?;
        if frame.samples() == 0 {
            return Ok(());
        }
        let format = AudioFormat::from_frame(frame)?;
        if self
            .resampler
            .as_ref()
            .is_some_and(|resampler| !resampler.format.matches(&format))
        {
            self.flush()?;
            self.resampler = None;
        }
        if self.resampler.is_none() {
            self.resampler = Some(Resampler::new(format, self.rate)?);
        }
        let mut planes = Vec::new();
        // SAFETY: all plane pointers and their sample counts come from the live
        // decoder-owned frame. swr_convert does not retain these pointers.
        unsafe {
            let planar = ffi::av_sample_fmt_is_planar(frame.format().into()) != 0;
            let channels = (*frame.as_ptr()).ch_layout.nb_channels as usize;
            let count = if planar { channels } else { 1 };
            if (*frame.as_ptr()).extended_data.is_null() {
                return Err("音频帧缺少采样数据".into());
            }
            for plane in 0..count {
                let pointer = *(*frame.as_ptr()).extended_data.add(plane);
                if pointer.is_null() {
                    return Err("音频帧缺少采样数据".into());
                }
                planes.push(pointer.cast_const());
            }
        }
        let converted = self
            .resampler
            .as_mut()
            .unwrap()
            .convert(planes.as_ptr(), frame.samples() as i32)?;
        self.write(&converted)
    }

    fn write(&mut self, samples: &[i16]) -> Result<(), String> {
        check_cancelled(&self.cancelled)?;
        if !samples.is_empty() {
            self.sink.write(samples)?;
            self.samples = self
                .samples
                .checked_add(samples.len() as u64)
                .ok_or("音频样本计数溢出")?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), String> {
        while let Some(resampler) = self.resampler.as_mut() {
            check_cancelled(&self.cancelled)?;
            let samples = resampler.convert(ptr::null(), 0)?;
            if samples.is_empty() {
                break;
            }
            self.write(&samples)?;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<AudioDecodeResult<S>, String> {
        self.flush()?;
        check_cancelled(&self.cancelled)?;
        self.sink.finish()?;
        Ok(AudioDecodeResult {
            sink: self.sink,
            samples: self.samples,
            has_audio: true,
        })
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod range_tests;

#[cfg(test)]
pub(crate) mod opus_tests;

#[cfg(test)]
mod opus_upload_tests;

#[cfg(test)]
mod flac_preview_tests;

#[cfg(test)]
mod file_test_helpers;

#[cfg(test)]
mod import_tests;

#[cfg(test)]
mod mp3_tests;
