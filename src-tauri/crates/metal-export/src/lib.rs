#![cfg(target_os = "macos")]

use native_overlay::{AssSource, ImageEvent, RenderInput, Renderer, SourceKind};
use serde_json::Value;
use std::{
    ffi::{c_char, c_void, CStr, CString},
    path::PathBuf,
};

mod backend;

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

unsafe fn create(
    config: *const c_char,
    device: *mut c_void,
    queue: *mut c_void,
    texture: *mut c_void,
) -> Result<Renderer, String> {
    let config: Value = serde_json::from_slice(CStr::from_ptr(config).to_bytes())
        .map_err(|error| error.to_string())?;
    let ass =
        std::fs::read_to_string(text(&config, "ass_file")?).map_err(|error| error.to_string())?;
    let mut images = Vec::new();
    for event in config["overlays"].as_array().ok_or("Missing overlays")? {
        let start = number(event, "start")?;
        let end = number(event, "end")?;
        images.push(ImageEvent {
            path: PathBuf::from(text(event, "path")?),
            start,
            end,
            from: (
                number(&event["from"], "x")? as f32,
                number(&event["from"], "y")? as f32,
            ),
            to: (
                number(&event["to"], "x")? as f32,
                number(&event["to"], "y")? as f32,
            ),
            width: number(event, "width")? as f32,
            height: number(event, "height")? as f32,
            opacity: number(event, "opacity")? as f32,
        });
    }
    Renderer::new_metal(
        1920,
        1080,
        RenderInput {
            documents: vec![AssSource {
                text: ass,
                kind: SourceKind::Danmaku,
            }],
            images,
        },
        device.cast(),
        queue.cast(),
        texture.cast(),
    )
}

#[no_mangle]
pub unsafe extern "C" fn skia_renderer_new(
    config: *const c_char,
    device: *mut c_void,
    queue: *mut c_void,
    texture: *mut c_void,
) -> *mut c_void {
    match create(config, device, queue, texture) {
        Ok(renderer) => Box::into_raw(Box::new(renderer)).cast(),
        Err(error) => {
            eprintln!("Skia initialization: {error}");
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn skia_renderer_draw(
    renderer: *mut c_void,
    time: f64,
    texture: *mut c_void,
) -> i32 {
    let renderer = &mut *renderer.cast::<Renderer>();
    match renderer.render_metal(time, texture.cast()) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Skia render: {error}");
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn skia_renderer_free(renderer: *mut c_void) {
    if !renderer.is_null() {
        drop(Box::from_raw(renderer.cast::<Renderer>()));
    }
}

/// H.264 / HEVC hardware-session availability in the selected FFmpeg adapter.
pub fn videotoolbox_codecs() -> Result<(bool, bool), String> {
    let mask = unsafe { (backend::get()?.probe)() };
    Ok((mask & 1 != 0, mask & 2 != 0))
}

/// Called before GUI/server startup when this executable is an export worker.
pub fn worker_entry() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--metal-export")) {
        return;
    }
    let Some(path) = args.next() else {
        std::process::exit(2)
    };
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = CString::new(path.as_bytes()) else {
        std::process::exit(2)
    };
    let backend = match backend::get() {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!("ERROR: {error}");
            std::process::exit(1)
        }
    };
    std::process::exit(unsafe { (backend.export)(path.as_ptr()) });
}
