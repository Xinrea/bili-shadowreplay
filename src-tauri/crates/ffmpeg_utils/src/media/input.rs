//! Native demuxer selection and per-input options for structured media jobs.
use super::{util, MediaInput};
use ffmpeg_next::{self as ffmpeg, ffi, format, Dictionary};
use std::{
    ffi::CString,
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

struct Opening(*mut ffi::AVFormatContext);
impl Drop for Opening {
    fn drop(&mut self) {
        unsafe { ffi::avformat_close_input(&mut self.0) };
    }
}

pub(super) fn open(
    source: &MediaInput,
    cancelled: Arc<AtomicBool>,
) -> Result<format::context::Input, String> {
    if source.format.is_none() && source.options.is_empty() {
        return crate::metadata::open_input(source.file_path()?, cancelled);
    }
    crate::metadata::initialize()?;
    util::check(&cancelled)?;
    let filename =
        CString::new(crate::path_str(source.file_path()?)?).map_err(|_| "输入路径包含 NUL")?;
    let demuxer = source
        .format
        .as_ref()
        .map(|name| {
            let name = CString::new(name.as_bytes()).map_err(|_| "输入格式包含 NUL")?;
            let found = unsafe { ffi::av_find_input_format(name.as_ptr()) };
            if found.is_null() {
                return Err("找不到指定的输入格式");
            }
            Ok(found)
        })
        .transpose()?
        .unwrap_or(ptr::null());
    let token = cancelled.clone();
    let interrupt = ffmpeg::util::interrupt::new(Box::new(move || token.load(Ordering::Acquire)));
    let mut context = Opening(unsafe { ffi::avformat_alloc_context() });
    if context.0.is_null() {
        return Err("分配媒体输入上下文失败".into());
    }
    unsafe {
        (*context.0).interrupt_callback = interrupt.interrupt;
    }
    let mut options = unsafe { util::options(&source.options)?.disown() };
    let result = unsafe {
        ffi::avformat_open_input(&mut context.0, filename.as_ptr(), demuxer, &mut options)
    };
    let options = unsafe { Dictionary::own(options) };
    if result < 0 {
        return Err(util::error("打开媒体输入失败", ffmpeg::Error::from(result)));
    }
    util::unused(&options, "输入封装器")?;
    let result = unsafe { ffi::avformat_find_stream_info(context.0, ptr::null_mut()) };
    util::check(&cancelled)?;
    if result < 0 {
        return Err(util::error(
            "读取媒体流信息失败",
            ffmpeg::Error::from(result),
        ));
    }
    let raw = std::mem::replace(&mut context.0, ptr::null_mut());
    Ok(unsafe { format::context::Input::wrap_with_interrupt(raw, interrupt.guard) })
}

/// Expand an explicit map-all selector after opening the real inputs, without
/// a second probe. Filter graph bindings deliberately remain single streams.
pub(super) fn expand_routes(
    routes: &[super::StreamRoute],
    inputs: &[Option<&format::context::Input>],
) -> Result<Vec<super::StreamRoute>, String> {
    use super::{StreamSelector, StreamSource};
    let mut expanded = Vec::new();
    for route in routes {
        let StreamSource::Input {
            input,
            selector: StreamSelector::AllAudio,
        } = route.source
        else {
            expanded.push(route.clone());
            continue;
        };
        let context = inputs
            .get(input)
            .and_then(|c| *c)
            .ok_or("全音轨映射必须引用文件输入")?;
        let mut found = false;
        for stream in context
            .streams()
            .filter(|s| s.parameters().medium() == ffmpeg::media::Type::Audio)
        {
            let mut route = route.clone();
            route.source = StreamSource::Input {
                input,
                selector: StreamSelector::Index(stream.index()),
            };
            expanded.push(route);
            found = true;
        }
        if !found && route.required {
            return Err(format!("输入{input}没有可映射的音轨"));
        }
    }
    Ok(expanded)
}

pub(super) fn progress_total(
    plan: &super::MediaPlan,
    inputs: &[Option<&format::context::Input>],
) -> Result<Option<f64>, String> {
    use super::{MediaProgressTotal, MediaSource};
    if let MediaProgressTotal::Known(value) = plan.progress_total {
        if !value.is_finite() || value < 0.0 {
            return Err("媒体进度总时长无效".into());
        }
        return Ok(Some(value));
    }
    let mut durations = Vec::new();
    for (index, source) in plan.inputs.iter().enumerate() {
        let mut duration = match &source.source {
            MediaSource::File(_) => inputs.get(index).and_then(|c| *c).and_then(|c| {
                (c.duration() > 0).then(|| c.duration() as f64 / f64::from(ffi::AV_TIME_BASE))
            }),
            MediaSource::RawVideo(spec) => Some(
                spec.frame_count as f64 * f64::from(spec.frame_rate_den)
                    / f64::from(spec.frame_rate_num),
            ),
            MediaSource::GeneratedVideo(_) | MediaSource::GeneratedAudio(_) => source.duration,
        };
        if let Some(seek) = source.seek {
            duration = duration.map(|d| (d - seek.seconds).max(0.0));
        }
        if let Some(limit) = source.duration {
            duration = Some(duration.map_or(limit, |d| d.min(limit)));
        }
        durations.push(duration);
    }
    let total = match plan.progress_total {
        MediaProgressTotal::FirstInput => durations.first().copied().flatten(),
        MediaProgressTotal::AllInputs => durations
            .into_iter()
            .try_fold(0.0, |total, next| next.map(|next| total + next)),
        MediaProgressTotal::Known(_) => unreachable!(),
    };
    Ok(plan
        .outputs
        .iter()
        .map(|output| {
            let mut duration = total.map(|d| (d - output.seek.unwrap_or(0.0)).max(0.0));
            if let Some(limit) = output.duration {
                duration = Some(duration.map_or(limit, |d| d.min(limit)));
            }
            duration
        })
        .collect::<Option<Vec<_>>>()
        .and_then(|v| v.into_iter().max_by(f64::total_cmp)))
}
