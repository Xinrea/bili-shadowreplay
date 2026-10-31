//! Stream-copy a live input into an event HLS playlist. The demuxer can be
//! interrupted by the recording switch; outputs remain alive to finish the
//! last segment before the caller closes the playlist.

use std::{
    ffi::CString,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use ffmpeg_next::{self as ffmpeg, codec, ffi, format, media, Dictionary, Packet, Rational};

use crate::{
    audio::remux::AudioPacketClock,
    metadata::{initialize, run_media},
    output::NativeOutput,
    path_str,
};

#[derive(Debug, Clone, Default)]
pub struct HlsRecordingOptions {
    pub user_agent: Option<String>,
    pub http_headers: Vec<(String, String)>,
    pub read_timeout: Option<Duration>,
}

/// Record the automatically selected video/audio streams, omitting subtitles.
/// Disabling `enabled` stops input I/O and finishes the current segment. Dropping
/// this future instead cancels all I/O; the blocking worker still owns its files.
pub async fn record_to_hls(
    source: &str,
    playlist: &Path,
    segment_pattern: &Path,
    options: HlsRecordingOptions,
    enabled: Arc<AtomicBool>,
) -> Result<(), String> {
    let source = source.to_owned();
    let playlist = playlist.to_owned();
    let segment_pattern = segment_pattern.to_owned();
    run_media(move |cancelled| {
        record_sync(
            &source,
            &playlist,
            &segment_pattern,
            options,
            enabled,
            cancelled,
        )
    })
    .await
}

fn record_sync(
    source: &str,
    playlist: &Path,
    segment_pattern: &Path,
    options: HlsRecordingOptions,
    enabled: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
) -> Result<(), String> {
    initialize()?;
    if !enabled.load(Ordering::Acquire) {
        return Ok(());
    }
    CString::new(source).map_err(|_| "录制地址包含 NUL".to_string())?;
    let mut input_options = Dictionary::new();
    if let Some(timeout) = options.read_timeout {
        input_options.set(
            "rw_timeout",
            &timeout.as_micros().min(i64::MAX as u128).to_string(),
        );
    }
    if let Some(agent) = options.user_agent {
        CString::new(agent.as_str()).map_err(|_| "User-Agent 包含 NUL".to_string())?;
        input_options.set("user_agent", &agent);
    }
    if !options.http_headers.is_empty() {
        let mut headers = String::new();
        for (name, value) in options.http_headers {
            headers.push_str(&name);
            headers.push_str(": ");
            headers.push_str(&value);
            headers.push_str("\r\n");
        }
        CString::new(headers.as_str()).map_err(|_| "HTTP 请求头包含 NUL".to_string())?;
        input_options.set("headers", &headers);
    }
    let input_cancel = cancelled.clone();
    let input_enabled = enabled.clone();
    let mut input = match format::input_with_interrupt_and_dictionary(
        source,
        move || input_cancel.load(Ordering::Acquire) || !input_enabled.load(Ordering::Acquire),
        input_options,
    ) {
        Ok(input) => input,
        Err(_) if !enabled.load(Ordering::Acquire) && !cancelled.load(Ordering::Acquire) => {
            return Ok(())
        }
        Err(error) => return Err(format!("打开录制输入失败: {error}")),
    };
    let chosen = [select_video(&input), crate::audio::audio_stream(&input)];
    let origin = unsafe { (*input.as_ptr()).start_time };
    let mut output = NativeOutput::new(playlist, "hls", cancelled.clone())?;
    // fftools' default mux delay gives MPEG-TS the same initial PCR/PTS margin.
    unsafe { (*output.output.as_mut_ptr()).max_delay = 700_000 };
    output.output.set_metadata(input.metadata().to_owned());
    let mut routes: Vec<Option<Route>> = (0..input.nb_streams()).map(|_| None).collect();
    for index in chosen.into_iter().flatten() {
        let stream = input.stream(index).ok_or("录制输入流不存在")?;
        let input_base = stream.time_base();
        if input_base.numerator() <= 0 || input_base.denominator() <= 0 {
            return Err("录制输入时基无效".into());
        }
        let parameters = stream.parameters().clone();
        let mut target = output
            .output
            .add_stream(ffmpeg::encoder::find(codec::Id::None))
            .map_err(|error| format!("创建录制输出流失败: {error}"))?;
        target.set_parameters(parameters.clone());
        target.set_time_base(input_base);
        target.set_metadata(stream.metadata().to_owned());
        unsafe {
            (*(*target.as_mut_ptr()).codecpar).codec_tag = 0;
            (*target.as_mut_ptr()).avg_frame_rate = (*stream.as_ptr()).avg_frame_rate;
            (*target.as_mut_ptr()).disposition = (*stream.as_ptr()).disposition;
        }
        let offset = if origin == ffi::AV_NOPTS_VALUE {
            0
        } else {
            unsafe {
                ffi::av_rescale_q(
                    origin.checked_neg().ok_or("录制时间戳溢出")?,
                    ffi::AVRational {
                        num: 1,
                        den: ffi::AV_TIME_BASE,
                    },
                    input_base.into(),
                )
            }
        };
        routes[index] = Some(Route {
            output_index: target.index(),
            input_base,
            parameters,
            offset,
            last_dts: None,
            audio_clock: (stream.parameters().medium() == media::Type::Audio)
                .then(AudioPacketClock::new),
        });
    }
    if routes.iter().all(Option::is_none) {
        return Err("录制输入没有可复制的音视频流".into());
    }
    let mut mux_options = Dictionary::new();
    mux_options.set("hls_time", "4");
    mux_options.set("hls_list_size", "0");
    mux_options.set("hls_playlist_type", "event");
    mux_options.set("hls_flags", "append_list+omit_endlist");
    mux_options.set("hls_segment_filename", path_str(segment_pattern)?);
    output
        .output
        .write_header_with(mux_options)
        .map_err(|error| format!("写入 HLS 文件头失败: {error}"))?;
    let mux_flags = unsafe { (*output.output.format().as_ptr()).flags };
    let read_result = loop {
        if cancelled.load(Ordering::Acquire) {
            return Err("录制已取消".into());
        }
        if !enabled.load(Ordering::Acquire) {
            break Ok(());
        }
        let mut packet = Packet::empty();
        match packet.read(&mut input) {
            Ok(()) => (),
            Err(ffmpeg::Error::Eof) => break Ok(()),
            Err(_) if !enabled.load(Ordering::Acquire) && !cancelled.load(Ordering::Acquire) => {
                break Ok(())
            }
            Err(ffmpeg::Error::Other { errno }) if errno == ffi::EAGAIN => {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(error) => break Err(format!("读取录制数据失败: {error}")),
        }
        let Some(route) = routes.get_mut(packet.stream()).and_then(Option::as_mut) else {
            continue;
        };
        for (value, pts) in [(packet.pts(), true), (packet.dts(), false)] {
            if let Some(value) = value {
                let value = value.checked_add(route.offset).ok_or("录制时间戳溢出")?;
                if pts {
                    packet.set_pts(Some(value));
                } else {
                    packet.set_dts(Some(value));
                }
            }
        }
        let output_base = output
            .output
            .stream(route.output_index)
            .ok_or("录制输出流不存在")?
            .time_base();
        match route.audio_clock.as_mut() {
            Some(clock) => clock.rescale(
                &mut packet,
                route.input_base,
                output_base,
                &route.parameters,
            )?,
            None => packet.rescale_ts(route.input_base, output_base),
        }
        packet.set_stream(route.output_index);
        packet.set_position(-1);
        // Like fftools and MediaPlan's muxer, repair live timestamp jitter in
        // the final output time base. Keep independent clocks for each stream.
        if mux_flags & ffi::AVFMT_NOTIMESTAMPS == 0 {
            if let (Some(dts), Some(last)) = (packet.dts(), route.last_dts) {
                let minimum = last
                    .checked_add(i64::from(mux_flags & ffi::AVFMT_TS_NONSTRICT == 0))
                    .ok_or("录制 DTS 溢出")?;
                if dts < minimum {
                    if let Some(pts) = packet.pts().filter(|pts| *pts >= dts) {
                        packet.set_pts(Some(pts.max(minimum)));
                    }
                    packet.set_dts(Some(minimum));
                }
            }
        }
        route.last_dts = packet.dts();
        packet
            .write_interleaved(&mut output.output)
            .map_err(|error| format!("写入录制分片失败: {error}"))?;
    };
    if cancelled.load(Ordering::Acquire) {
        return Err("录制已取消".into());
    }
    // A read failure leaves the local muxer writable. Publish its pending tail
    // before reporting that failure so a reconnect can append without reusing it.
    let trailer_result = output
        .output
        .write_trailer()
        .map_err(|error| format!("结束 HLS 录制失败: {error}"));
    let close_result = output.close();
    read_result?;
    trailer_result?;
    close_result
}

struct Route {
    output_index: usize,
    input_base: Rational,
    parameters: codec::Parameters,
    offset: i64,
    last_dts: Option<i64>,
    audio_clock: Option<AudioPacketClock>,
}

fn select_video(input: &format::context::Input) -> Option<usize> {
    input
        .streams()
        .filter_map(|stream| unsafe {
            let raw = &*stream.as_ptr();
            if stream.parameters().medium() != media::Type::Video
                || raw.discard == ffi::AVDiscard::AVDISCARD_ALL
            {
                return None;
            }
            let score = if raw.disposition & ffi::AV_DISPOSITION_ATTACHED_PIC != 0 {
                1
            } else {
                i64::from((*raw.codecpar).width) * i64::from((*raw.codecpar).height)
                    + if raw.event_flags & ffi::AVSTREAM_EVENT_FLAG_NEW_PACKETS != 0 {
                        100_000_000
                    } else {
                        0
                    }
                    + if raw.disposition & ffi::AV_DISPOSITION_DEFAULT != 0 {
                        5_000_000
                    } else {
                        0
                    }
            };
            Some((stream.index(), score))
        })
        .fold(None, |best: Option<(usize, i64)>, candidate| {
            if best.is_none_or(|(_, score)| candidate.1 > score) {
                Some(candidate)
            } else {
                best
            }
        })
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests;
