//! Deliberate test-only GPU readback against a double precision colour oracle.
use super::*;

fn allocate(pool: &FramePool) -> frame::Video {
    let mut gpu = frame::Video::empty();
    unsafe {
        check(
            ffi::av_hwframe_get_buffer(pool.raw.0, gpu.as_mut_ptr(), 0),
            "test allocation",
        )
        .unwrap();
    }
    gpu
}

fn upload(pool: &FramePool, cpu: &frame::Video) -> frame::Video {
    let mut gpu = allocate(pool);
    unsafe {
        check(
            ffi::av_hwframe_transfer_data(gpu.as_mut_ptr(), cpu.as_ptr(), 0),
            "test upload",
        )
        .unwrap();
    }
    gpu
}

fn bt709_from_srgb(encoded: f64) -> f64 {
    let s = encoded.clamp(0.0, 1.0);
    let linear = if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    };
    if linear < 0.018 {
        4.5 * linear
    } else {
        1.099 * linear.powf(0.45) - 0.099
    }
}

fn yuv(rgb: [f64; 3]) -> [f64; 3] {
    let [r, g, b] = rgb;
    [
        16.0 + 219.0 * (0.2126 * r + 0.7152 * g + 0.0722 * b),
        128.0 + 224.0 * (-0.11457210605734 * r - 0.38542789394266 * g + 0.5 * b),
        128.0 + 224.0 * (0.5 * r - 0.454152908305817 * g - 0.045847091694183 * b),
    ]
}

#[test]
#[ignore = "requires an accessible Vulkan GPU (MoltenVK on macOS)"]
fn gpu_compute_converts_srgb_before_premultiplied_blending() {
    crate::metadata::initialize().unwrap();
    let options = VulkanRenderOptions {
        width: 18,
        height: 10,
        ..Default::default()
    };
    let device = VulkanDevice::new_with_mode("", VulkanSubmissionMode::Asynchronous).unwrap();
    let rgba_pool = FramePool::new(device.clone(), &options).unwrap();
    let nv12_pool = FramePool::new_nv12(device.clone(), &options, false).unwrap();
    let mut compute = ComputeCompositor::new(device, 3).unwrap();
    let mut pending = Vec::new();
    let mut samples = 0;
    let mut maximum_error = 0;
    // Adjacent values straddle both transfer-function knees. The second base
    // covers coloured chroma, including out-of-gamut and out-of-range samples.
    for case in 0..38 {
        let mut base = frame::Video::new(ffmpeg::format::Pixel::NV12, 18, 10);
        let ys = [
            0, 16, 17, 20, 24, 25, 26, 46, 47, 48, 64, 80, 96, 128, 160, 200, 235, 255,
        ];
        for y in 0..10 {
            for x in 0..18 {
                let offset = y * base.stride(0) + x;
                base.data_mut(0)[offset] = ys[(x + y * 3) % ys.len()];
            }
        }
        for y in 0..5 {
            for x in 0..18 {
                let offset = y * base.stride(1) + x;
                base.data_mut(1)[offset] = if case < 19 {
                    128
                } else {
                    (16 + (x * 19 + y * 23) % 225) as u8
                };
            }
        }
        let mut rgba = frame::Video::new(ffmpeg::format::Pixel::RGBA, 18, 10);
        for y in 0..10 {
            for x in 0..18 {
                let kind = case % 19;
                let color = [
                    [255, 0, 0],
                    [0, 255, 0],
                    [0, 0, 255],
                    [255, 255, 255],
                    [0, 0, 0],
                    [53, 179, 241],
                ][kind % 6];
                let alpha = match kind {
                    0..=5 => 0,
                    6..=11 => 255,
                    12..=17 => 128,
                    _ => [0, 64, 128, 255][(y % 2) * 2 + x % 2],
                };
                let offset = y * rgba.stride(0) + x * 4;
                for (c, value) in color.into_iter().enumerate() {
                    rgba.data_mut(0)[offset + c] = ((value * alpha + 127) / 255) as u8;
                }
                rgba.data_mut(0)[offset + 3] = alpha as u8;
            }
        }
        let overlay = upload(&rgba_pool, &rgba);
        let gpu_base = upload(&nv12_pool, &base);
        let output = allocate(&nv12_pool);
        compute
            .submit(&overlay, Some(&gpu_base), &output, SdrTransfer::Srgb)
            .unwrap();
        pending.push((case, base, rgba, output));
        if pending.len() < 3 && case != 37 {
            continue;
        }
        for (case, base, rgba, output) in pending.drain(..) {
            let mut actual = frame::Video::empty();
            unsafe {
                check(
                    ffi::av_hwframe_transfer_data(actual.as_mut_ptr(), output.as_ptr(), 0),
                    "test-only download",
                )
                .unwrap();
            }
            let pixel = |x: usize, y: usize| {
                let luma = (base.data(0)[y * base.stride(0) + x] as f64 - 16.0) / 219.0;
                let uv = y / 2 * base.stride(1) + x / 2 * 2;
                let cb = (base.data(1)[uv] as f64 - 128.0) / 224.0;
                let cr = (base.data(1)[uv + 1] as f64 - 128.0) / 224.0;
                let background = [
                    luma + 1.5748 * cr,
                    luma - 0.187324272930649 * cb - 0.468124272930649 * cr,
                    luma + 1.8556 * cb,
                ]
                .map(bt709_from_srgb);
                let offset = y * rgba.stride(0) + x * 4;
                let alpha = rgba.data(0)[offset + 3] as f64 / 255.0;
                let mut blended = [0.0; 3];
                for c in 0..3 {
                    // Unpremultiply the foreground, blend in output transfer
                    // space. This also catches accidentally converting alpha
                    // or treating Skia's overlay as source-space sRGB.
                    let foreground = if alpha == 0.0 {
                        0.0
                    } else {
                        rgba.data(0)[offset + c] as f64 / 255.0 / alpha
                    };
                    blended[c] = background[c] * (1.0 - alpha) + foreground * alpha;
                }
                yuv(blended)
            };
            for plane in 0..2 {
                for y in 0..if plane == 0 { 10 } else { 5 } {
                    for x in 0..18 {
                        let expected = if plane == 0 {
                            pixel(x, y)[0]
                        } else {
                            (0..2)
                                .flat_map(|dy| (0..2).map(move |dx| (dx, dy)))
                                .map(|(dx, dy)| pixel(x / 2 * 2 + dx, y * 2 + dy)[1 + x % 2] * 0.25)
                                .sum::<f64>()
                        }
                        .round() as i32;
                        let actual = actual.data(plane)[y * actual.stride(plane) + x] as i32;
                        let error = (actual - expected).abs();
                        maximum_error = maximum_error.max(error);
                        assert!(
                            error <= 1,
                            "case={case} plane={plane} ({x},{y}): {actual} vs {expected}"
                        );
                        samples += 1;
                    }
                }
            }
        }
    }
    compute.drain().unwrap();
    println!("sRGB GPU oracle: 38 frames, {samples} samples, maximum byte error {maximum_error}");
}
