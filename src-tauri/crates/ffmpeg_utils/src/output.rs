//! Native output I/O shared by media encoders and muxers. The interrupt closure
//! outlives AVIO, and explicit close reports buffered write failures.

use std::{
    ffi::CString,
    path::Path,
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use ffmpeg_next::{self as ffmpeg, ffi, format};

use crate::path_str;

pub(crate) struct NativeOutput {
    pub(crate) output: format::context::Output,
    _interrupt: ffmpeg::util::interrupt::InterruptGuard,
}

impl NativeOutput {
    pub(crate) fn new(
        path: &Path,
        format: &str,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let filename = CString::new(path_str(path)?).map_err(|error| error.to_string())?;
        let format = CString::new(format).map_err(|error| error.to_string())?;
        let interrupt =
            ffmpeg::util::interrupt::new(Box::new(move || cancelled.load(Ordering::Acquire)));
        let mut context = ptr::null_mut();
        let result = unsafe {
            ffi::avformat_alloc_output_context2(
                &mut context,
                ptr::null(),
                format.as_ptr(),
                filename.as_ptr(),
            )
        };
        if result < 0 || context.is_null() {
            return Err(native_error(
                "创建媒体输出失败",
                ffmpeg::Error::from(result),
            ));
        }
        // Wrap before opening AVIO, so failed opens still free the context.
        let output = unsafe { format::context::Output::wrap(context) };
        unsafe {
            (*context).interrupt_callback = interrupt.interrupt;
            // Muxers such as HLS own their playlists and segment I/O. Opening
            // an extra outer file would truncate the existing event playlist.
            if !output.format().flags().contains(format::flag::Flags::NO_FILE) {
                let result = ffi::avio_open2(
                    &mut (*context).pb,
                    filename.as_ptr(),
                    ffi::AVIO_FLAG_WRITE,
                    &(*context).interrupt_callback,
                    ptr::null_mut(),
                );
                if result < 0 {
                    return Err(native_error("打开媒体输出失败", ffmpeg::Error::from(result)));
                }
            }
        }
        Ok(Self {
            output,
            _interrupt: interrupt.guard,
        })
    }

    pub(crate) fn close(&mut self) -> Result<(), String> {
        unsafe {
            let context = self.output.as_mut_ptr();
            if (*context).pb.is_null() {
                return Ok(());
            }
            ffi::avio_flush((*context).pb);
            let error = (*(*context).pb).error;
            let result = ffi::avio_closep(&mut (*context).pb);
            if error < 0 || result < 0 {
                return Err(native_error(
                    "关闭媒体输出失败",
                    ffmpeg::Error::from(if error < 0 { error } else { result }),
                ));
            }
        }
        Ok(())
    }
}

fn native_error(context: &str, error: ffmpeg::Error) -> String {
    format!("{context}: {error}")
}
