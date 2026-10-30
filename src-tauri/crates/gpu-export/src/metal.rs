use super::*;

#[no_mangle]
pub unsafe extern "C" fn bsr_skia_metal_new(
    config: *const c_char,
    width: u32,
    height: u32,
    device: *mut c_void,
    queue: *mut c_void,
    texture: *mut c_void,
) -> *mut c_void {
    new_renderer(scene(config).and_then(|scene| {
        Renderer::new_metal(
            width,
            height,
            scene.input,
            device.cast(),
            queue.cast(),
            texture.cast(),
        )
        .map(|renderer| (renderer, scene.start))
    }))
}

#[no_mangle]
pub unsafe extern "C" fn bsr_skia_metal_draw(
    renderer: *mut c_void,
    time: f64,
    texture: *mut c_void,
) -> i32 {
    let renderer = &mut *renderer.cast::<GpuRenderer>();
    draw_result(
        renderer
            .renderer
            .render_metal(time + renderer.start, texture.cast()),
    )
}
