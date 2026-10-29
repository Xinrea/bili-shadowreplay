#![cfg(any(
    target_os = "macos",
    all(feature = "vulkan", any(target_os = "linux", target_os = "windows"))
))]

use native_overlay::{AssSource, ImageEvent, RenderInput, Renderer, SourceKind};
use serde_json::{json, Value};
use std::{
    ffi::{c_char, c_void, CStr, CString},
    io::Write,
    path::PathBuf,
};

mod backend;
#[cfg(target_os = "macos")]
mod metal;
#[cfg(all(feature = "vulkan", any(target_os = "linux", target_os = "windows")))]
mod vulkan;

#[repr(C)]
struct EncoderOption {
    name: *const c_char,
    value: *const c_char,
}

#[repr(C)]
pub(crate) struct ExportConfig {
    source: *const c_char,
    output: *const c_char,
    scene: *const c_char,
    fps: f64,
    bitrate: i64,
    hevc: i32,
    options: *const EncoderOption,
    option_count: usize,
    prepared: i32,
}

fn number(value: &Value, key: &str) -> Result<f64, String> {
    value[key]
        .as_f64()
        .ok_or_else(|| format!("Missing number: {key}"))
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("Missing string: {key}"))
}

struct Scene {
    input: RenderInput,
    start: f64,
}

unsafe fn scene(config: *const c_char) -> Result<Scene, String> {
    if config.is_null() {
        return Ok(Scene {
            input: RenderInput::default(),
            start: 0.0,
        });
    }
    let config: Value =
        serde_json::from_slice(CStr::from_ptr(config).to_bytes()).map_err(|e| e.to_string())?;
    let images = config["overlays"]
        .as_array()
        .ok_or("Missing overlays")?
        .iter()
        .map(|v| {
            Ok(ImageEvent {
                path: PathBuf::from(text(v, "path")?),
                start: number(v, "start")?,
                end: number(v, "end")?,
                from: (
                    number(&v["from"], "x")? as f32,
                    number(&v["from"], "y")? as f32,
                ),
                to: (number(&v["to"], "x")? as f32, number(&v["to"], "y")? as f32),
                width: number(v, "width")? as f32,
                height: number(v, "height")? as f32,
                opacity: number(v, "opacity")? as f32,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let documents = config["documents"]
        .as_array()
        .ok_or("Missing documents")?
        .iter()
        .map(|document| {
            Ok(AssSource {
                text: std::fs::read_to_string(text(document, "path")?)
                    .map_err(|e| e.to_string())?,
                kind: match text(document, "kind")? {
                    "subtitle" => SourceKind::Subtitle,
                    "danmaku" => SourceKind::Danmaku,
                    _ => return Err("Invalid ASS source kind".into()),
                },
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Scene {
        input: RenderInput { documents, images },
        start: number(&config, "start")?,
    })
}

struct GpuRenderer {
    renderer: Renderer,
    start: f64,
}

fn new_renderer(result: Result<(Renderer, f64), String>) -> *mut c_void {
    match result {
        Ok((renderer, start)) => Box::into_raw(Box::new(GpuRenderer { renderer, start })).cast(),
        Err(error) => {
            eprintln!("Skia initialization: {error}");
            std::ptr::null_mut()
        }
    }
}
fn draw_result(result: Result<(), String>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Skia render: {error}");
            -1
        }
    }
}

/// # Safety
/// `renderer` must be null or a live handle returned by a Skia backend constructor.
/// All GPU work using it must be retired before this call.
#[no_mangle]
pub unsafe extern "C" fn bsr_skia_free(renderer: *mut c_void) {
    if !renderer.is_null() {
        drop(Box::from_raw(renderer.cast::<GpuRenderer>()));
    }
}
#[no_mangle]
pub extern "C" fn bsr_gpu_progress(frames: i64, time: f64, elapsed: f64, done: i32) {
    let value = if done != 0 {
        json!({"ok":true,"frames":frames,"total_seconds":elapsed})
    } else {
        json!({"time_seconds":time,"elapsed_seconds":elapsed})
    };
    let mut output = std::io::stdout().lock();
    let _ = writeln!(output, "{value}");
    let _ = output.flush();
}

fn native_error(error: &[c_char]) -> String {
    unsafe { CStr::from_ptr(error.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

/// Probe real GPU frames and return (available, maximum quality level) for H.264/HEVC.
pub fn codecs() -> Result<[(bool, i32); 2], String> {
    let mut error = [0 as c_char; 2048];
    let mut quality_max = [0; 2];
    let mask = unsafe {
        (backend::get()?.probe)(error.as_mut_ptr(), error.len(), quality_max.as_mut_ptr())
    };
    if mask < 0 {
        return Err(native_error(&error));
    }
    Ok([
        (mask & 1 != 0, quality_max[0]),
        (mask & 2 != 0, quality_max[1]),
    ])
}

/// Runs before GUI/server initialization; dropping the parent task kills it.
pub fn worker_entry() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--gpu-export")) {
        return;
    }
    let mut run = || -> Result<i32, String> {
        let path = args.next().ok_or("Missing GPU export job")?;
        let config: Value =
            serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let source = CString::new(text(&config, "source")?).map_err(|e| e.to_string())?;
        let output = CString::new(text(&config, "output")?).map_err(|e| e.to_string())?;
        let encoder = text(&config, "encoder")?;
        let expected = if cfg!(target_os = "macos") {
            "videotoolbox"
        } else {
            "vulkan"
        };
        let hevc = match encoder.strip_suffix(&format!("_{expected}")) {
            Some("h264") => 0,
            Some("hevc") => 1,
            _ => return Err(format!("Expected a {expected} hardware encoder")),
        };
        let scene = CString::new(config.to_string()).map_err(|e| e.to_string())?;
        let strings = config["options"]
            .as_object()
            .ok_or("Missing encoder options")?
            .iter()
            .map(|(name, value)| {
                Ok((
                    CString::new(name.as_str()).map_err(|e| e.to_string())?,
                    CString::new(value.as_str().ok_or("Encoder options must be strings")?)
                        .map_err(|e| e.to_string())?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let options: Vec<_> = strings
            .iter()
            .map(|(name, value)| EncoderOption {
                name: name.as_ptr(),
                value: value.as_ptr(),
            })
            .collect();
        let job = ExportConfig {
            source: source.as_ptr(),
            output: output.as_ptr(),
            scene: scene.as_ptr(),
            fps: number(&config, "fps")?,
            bitrate: config["bitrate_bps"].as_i64().ok_or("Missing bitrate")?,
            hevc,
            options: options.as_ptr(),
            option_count: options.len(),
            prepared: i32::from(
                config["prepared"]
                    .as_bool()
                    .ok_or("Missing prepared input flag")?,
            ),
        };
        let mut error = [0 as c_char; 2048];
        let code = unsafe { (backend::get()?.export)(&job, error.as_mut_ptr(), error.len()) };
        if code != 0 {
            eprintln!("GPU export: {}", native_error(&error));
        }
        Ok(code)
    };
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("GPU export: {error}");
            std::process::exit(1)
        }
    }
}
