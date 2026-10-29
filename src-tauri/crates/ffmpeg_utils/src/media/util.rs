use super::AvOptions;
use ffmpeg_next::{self as ffmpeg, codec, ffi, format, Dictionary};
use std::{
    ffi::CString,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) const MICROS: ffi::AVRational = ffi::AVRational {
    num: 1,
    den: ffi::AV_TIME_BASE,
};
pub(super) fn error(context: &str, value: impl std::fmt::Display) -> String {
    format!("{context}: {value}")
}
pub(super) fn check(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Acquire) {
        Err("媒体处理已取消".into())
    } else {
        Ok(())
    }
}
pub(super) fn again(value: ffmpeg::Error) -> bool {
    matches!(value, ffmpeg::Error::Other { errno } if errno == ffi::EAGAIN)
}
pub(super) fn time(value: f64) -> Result<i64, String> {
    if !value.is_finite() || value < 0.0 {
        return Err("媒体时间必须是非负有限数".into());
    }
    let value = CString::new(value.to_string()).map_err(|e| e.to_string())?;
    let mut result = 0;
    if unsafe { ffi::av_parse_time(&mut result, value.as_ptr(), 1) } < 0 || result < 0 {
        return Err("媒体时间超出范围".into());
    }
    Ok(result)
}
pub(super) fn options(values: &AvOptions) -> Result<Dictionary<'static>, String> {
    let mut output = Dictionary::new();
    for (key, value) in values {
        if key.is_empty() || key.as_bytes().contains(&0) || value.as_bytes().contains(&0) {
            return Err("AVOption 名称或值无效".into());
        }
        output.set(key, value);
    }
    Ok(output)
}
pub(super) fn unused(values: &Dictionary<'_>, stage: &str) -> Result<(), String> {
    let keys = values.iter().map(|(key, _)| key).collect::<Vec<_>>();
    if keys.is_empty() {
        Ok(())
    } else {
        Err(format!("{stage} 不支持 AVOptions: {}", keys.join(", ")))
    }
}
pub(super) fn open_codec(
    context: &mut codec::Context,
    codec: ffmpeg::Codec,
    values: &AvOptions,
) -> Result<(), String> {
    let mut dictionary = unsafe { options(values)?.disown() };
    let result =
        unsafe { ffi::avcodec_open2(context.as_mut_ptr(), codec.as_ptr(), &mut dictionary) };
    let dictionary = unsafe { Dictionary::own(dictionary) };
    if result < 0 {
        return Err(error("打开媒体编码器失败", ffmpeg::Error::from(result)));
    }
    unused(&dictionary, "编码器")
}
pub(super) fn validate_options(codec: ffmpeg::Codec, values: &AvOptions) -> Result<(), String> {
    let mut context = codec::Context::new_with_codec(codec);
    for key in values.keys() {
        let key_c = CString::new(key.as_bytes()).map_err(|e| e.to_string())?;
        if unsafe {
            ffi::av_opt_find(
                context.as_mut_ptr().cast(),
                key_c.as_ptr(),
                ptr::null(),
                0,
                ffi::AV_OPT_SEARCH_CHILDREN,
            )
        }
        .is_null()
        {
            return Err(format!("编码器 {} 不支持 AVOption {key}", codec.name()));
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub input_start: Option<i64>,
    pub input_duration: Option<i64>,
    pub output_start: i64,
    pub output_seek: bool,
    pub output_duration: Option<i64>,
}
impl Limits {
    pub(super) fn trim(&self, audio: bool, input: bool) -> Option<String> {
        let (start, duration) = if input {
            (self.input_start, self.input_duration)
        } else {
            (
                self.output_seek.then_some(self.output_start),
                self.output_duration,
            )
        };
        if start.is_none() && duration.is_none() {
            return None;
        }
        let name = if audio { "atrim" } else { "trim" };
        let mut args = Vec::new();
        if let Some(start) = start {
            args.push(format!("starti={start}us"));
        }
        if let Some(duration) = duration {
            if duration == 0 {
                args.push(format!("endi={}us", start.unwrap_or(0)));
            } else {
                args.push(format!("durationi={duration}us"));
            }
        }
        Some(format!("{name}={}", args.join(":")))
    }
    pub(super) fn encode(&self, pts: i64, base: ffi::AVRational) -> bool {
        self.output_duration
            .is_none_or(|duration| unsafe { ffi::av_compare_ts(pts, base, duration, MICROS) } < 0)
    }
}

pub(super) struct Device {
    pub raw: *mut ffi::AVBufferRef,
}
impl Device {
    pub(super) fn new(options: &super::HardwareDevice) -> Result<Self, String> {
        let kind = CString::new(options.kind.as_bytes()).map_err(|e| e.to_string())?;
        let kind = unsafe { ffi::av_hwdevice_find_type_by_name(kind.as_ptr()) };
        if kind == ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_NONE {
            return Err(format!("未知硬件设备类型 {}", options.kind));
        }
        let path = options
            .device
            .as_ref()
            .map(|s| CString::new(s.as_bytes()))
            .transpose()
            .map_err(|e| e.to_string())?;
        let mut raw = ptr::null_mut();
        let result = unsafe {
            ffi::av_hwdevice_ctx_create(
                &mut raw,
                kind,
                path.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                ptr::null_mut(),
                0,
            )
        };
        if result < 0 {
            return Err(error("创建硬件设备失败", ffmpeg::Error::from(result)));
        }
        Ok(Self { raw })
    }
    pub(super) fn attach(&self, graph: &mut ffmpeg::filter::Graph) -> Result<(), String> {
        unsafe {
            let raw = &mut *graph.as_mut_ptr();
            for index in 0..raw.nb_filters {
                let context = &mut **raw.filters.add(index as usize);
                if context.hw_device_ctx.is_null() {
                    context.hw_device_ctx = ffi::av_buffer_ref(self.raw);
                    if context.hw_device_ctx.is_null() {
                        return Err("复制硬件设备引用失败".into());
                    }
                }
            }
        }
        Ok(())
    }
}
impl Drop for Device {
    fn drop(&mut self) {
        unsafe { ffi::av_buffer_unref(&mut self.raw) };
    }
}

pub(super) fn side_data(
    source: &format::stream::Stream<'_>,
    target: &mut format::stream::StreamMut<'_>,
    skip_matrix: bool,
) -> Result<(), String> {
    for side in source.side_data() {
        if skip_matrix && side.kind() == ffmpeg::packet::side_data::Type::DisplayMatrix {
            continue;
        }
        #[cfg(bsr_ffmpeg_8_0)]
        unsafe {
            let parameters = &mut *(*target.as_mut_ptr()).codecpar;
            if !ffi::av_packet_side_data_get(
                parameters.coded_side_data,
                parameters.nb_coded_side_data,
                side.kind().into(),
            )
            .is_null()
            {
                continue;
            }
            let destination = ffi::av_packet_side_data_new(
                &mut parameters.coded_side_data,
                &mut parameters.nb_coded_side_data,
                side.kind().into(),
                side.data().len(),
                0,
            );
            if destination.is_null() {
                return Err("分配媒体侧数据失败".into());
            }
            ptr::copy_nonoverlapping(side.data().as_ptr(), (*destination).data, side.data().len());
        }
        #[cfg(not(bsr_ffmpeg_8_0))]
        unsafe {
            let data = ffi::av_malloc(side.data().len()).cast::<u8>();
            if data.is_null() {
                return Err("分配媒体侧数据失败".into());
            }
            ptr::copy_nonoverlapping(side.data().as_ptr(), data, side.data().len());
            let result = ffi::av_stream_add_side_data(
                target.as_mut_ptr(),
                side.kind().into(),
                data,
                side.data().len(),
            );
            if result < 0 {
                ffi::av_free(data.cast());
                return Err(error("复制媒体侧数据失败", ffmpeg::Error::from(result)));
            }
        }
    }
    Ok(())
}
