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
        return crate::metadata::open_input(&source.path, cancelled);
    }
    crate::metadata::initialize()?;
    util::check(&cancelled)?;
    let filename = CString::new(crate::path_str(&source.path)?).map_err(|_| "输入路径包含 NUL")?;
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
