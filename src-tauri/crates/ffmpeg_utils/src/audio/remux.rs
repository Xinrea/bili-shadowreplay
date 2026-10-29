//! Single-audio-stream copying. Packet payload and side data remain intact;
//! only timestamps and the output container change.

use std::{
    path::Path,
    sync::{atomic::AtomicBool, Arc},
};

use ffmpeg_next::packet::Mut;
use ffmpeg_next::{self as ffmpeg, codec, ffi, media, Dictionary, Packet, Rational};
use tempfile::NamedTempFile;

use super::{
    check_cancelled,
    encode::{copy_chapters, input_metadata},
    native_error,
    output::NativeOutput,
};
use crate::metadata::{open_input, run_media};

pub(super) enum CopyAudioFormat {
    Wav,
    Flac,
    M4a,
}

impl CopyAudioFormat {
    fn muxer(&self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::M4a => "ipod",
        }
    }
}

/// Per-stream rescaling state used by fftools' audio streamcopy. Callers apply
/// their demux timestamp offset before this mapping. Coarse FLV clocks need
/// sample-grid recovery rather than independently rounding every packet.
pub(crate) struct AudioPacketClock {
    last: i64,
}

impl AudioPacketClock {
    pub(crate) fn new() -> Self {
        Self {
            last: ffi::AV_NOPTS_VALUE,
        }
    }

    pub(crate) fn rescale(
        &mut self,
        packet: &mut Packet,
        input_base: Rational,
        output_base: Rational,
        parameters: &codec::Parameters,
    ) -> Result<(), String> {
        let raw = unsafe { &*parameters.as_ptr() };
        if input_base.numerator() <= 0
            || input_base.denominator() <= 0
            || output_base.numerator() <= 0
            || output_base.denominator() <= 0
            || raw.sample_rate <= 0
        {
            return Err("音频包时基或采样率无效".into());
        }
        let mut duration = unsafe {
            ffi::av_get_audio_frame_duration2(parameters.as_ptr().cast_mut(), packet.size() as i32)
        };
        if duration == 0 {
            duration = raw.frame_size;
        }
        if duration < 0 {
            return Err("音频包样本长度无效".into());
        }
        if let Some(dts) = packet.dts() {
            let dts = unsafe {
                ffi::av_rescale_delta(
                    input_base.into(),
                    dts,
                    ffi::AVRational {
                        num: 1,
                        den: raw.sample_rate,
                    },
                    duration,
                    &mut self.last,
                    output_base.into(),
                )
            };
            packet.set_dts(Some(dts));
            packet.set_pts(Some(dts));
            let duration = unsafe {
                ffi::av_rescale_q(packet.duration(), input_base.into(), output_base.into())
            };
            packet.set_duration(duration);
        } else {
            packet.rescale_ts(input_base, output_base);
        }
        unsafe {
            (*packet.as_mut_ptr()).time_base = output_base.into();
        }
        Ok(())
    }
}

pub(super) async fn remux_first_audio(
    input: &Path,
    output: &Path,
    format: CopyAudioFormat,
) -> Result<(), String> {
    let input = input.to_path_buf();
    let output = output.to_path_buf();
    let publish = output.clone();
    let temporary =
        run_media(move |cancelled| remux_sync(&input, &output, format, cancelled)).await?;
    temporary
        .persist(publish)
        .map_err(|error| native_error("发布音频文件失败", error.error))?;
    Ok(())
}

fn remux_sync(
    input_path: &Path,
    output_path: &Path,
    format: CopyAudioFormat,
    cancelled: Arc<AtomicBool>,
) -> Result<NamedTempFile, String> {
    if input_path == output_path
        || std::fs::canonicalize(input_path)
            .ok()
            .zip(std::fs::canonicalize(output_path).ok())
            .is_some_and(|(input, output)| input == output)
    {
        return Err("音频输出不能覆盖输入".into());
    }
    let mut input = open_input(input_path, cancelled.clone())?;
    let index = input
        .streams()
        .find(|stream| stream.parameters().medium() == media::Type::Audio)
        .map(|stream| stream.index())
        .ok_or("输入没有音轨")?;
    let origin = unsafe { (*input.as_ptr()).start_time };
    let stream = input.stream(index).ok_or("找不到音轨")?;
    let input_base = stream.time_base();
    let parameters = stream.parameters();
    let (global, metadata) = input_metadata(&input, index)?;
    let parent = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = tempfile::Builder::new()
        .prefix(".audio-remux-")
        .suffix(if matches!(format, CopyAudioFormat::M4a) {
            ".m4a"
        } else {
            ".tmp"
        })
        .tempfile_in(parent)
        .map_err(|error| native_error("创建音频复制临时文件失败", error))?;
    let mut output = NativeOutput::new(temporary.path(), format.muxer(), cancelled.clone())?;
    output.output.set_metadata(global);
    let mut target = output
        .output
        .add_stream(ffmpeg::encoder::find(codec::Id::None))
        .map_err(|error| native_error("创建复制音轨失败", error))?;
    target.set_parameters(parameters.clone());
    target.set_time_base(input_base);
    target.set_metadata(metadata);
    unsafe {
        (*(*target.as_mut_ptr()).codecpar).codec_tag = 0;
        (*target.as_mut_ptr()).disposition = (*stream.as_ptr()).disposition;
        (*target.as_mut_ptr()).avg_frame_rate = (*stream.as_ptr()).avg_frame_rate;
    }
    copy_chapters(&input, &mut output.output, origin)?;
    let mut options = Dictionary::new();
    if matches!(format, CopyAudioFormat::M4a) {
        options.set("movflags", "+faststart");
    }
    // AVFMT_FLAG_AUTO_BSF lets the MP4 muxer insert aac_adtstoasc for ADTS
    // sources, preserving codec configuration without running a decoder.
    output
        .output
        .write_header_with(options)
        .map_err(|error| native_error("写入音频文件头失败", error))?;
    let output_base = output.output.stream(0).ok_or("复制音轨不存在")?.time_base();
    let offset = if origin == ffi::AV_NOPTS_VALUE {
        0
    } else {
        unsafe {
            ffi::av_rescale_q(
                origin.checked_neg().ok_or("音频时间戳溢出")?,
                ffi::AVRational {
                    num: 1,
                    den: ffi::AV_TIME_BASE,
                },
                input_base.into(),
            )
        }
    };
    let mut clock = AudioPacketClock::new();
    loop {
        check_cancelled(&cancelled)?;
        let mut packet = Packet::empty();
        match packet.read(&mut input) {
            Ok(()) => (),
            Err(ffmpeg::Error::Eof) => break,
            Err(error) => return Err(native_error("读取音频复制包失败", error)),
        }
        if packet.stream() != index {
            continue;
        }
        if let Some(pts) = packet.pts() {
            packet.set_pts(Some(pts.checked_add(offset).ok_or("音频时间戳溢出")?));
        }
        if let Some(dts) = packet.dts() {
            packet.set_dts(Some(dts.checked_add(offset).ok_or("音频时间戳溢出")?));
        }
        clock.rescale(&mut packet, input_base, output_base, &parameters)?;
        packet.set_stream(0);
        packet.set_position(-1);
        packet
            .write_interleaved(&mut output.output)
            .map_err(|error| native_error("写入音频复制包失败", error))?;
    }
    check_cancelled(&cancelled)?;
    output
        .output
        .write_trailer()
        .map_err(|error| native_error("写入音频文件尾失败", error))?;
    output.close()?;
    drop(output);
    check_cancelled(&cancelled)?;
    Ok(temporary)
}
