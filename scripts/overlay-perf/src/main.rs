//! Fixed-scene CPU benchmark; run through ../run.py rather than Cargo directly.
use native_overlay::{AssSource, ImageEvent, RenderInput, Renderer, SourceKind};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{hint::black_box, path::Path, time::Instant};

#[cfg(feature = "current")]
#[allow(dead_code)]
#[path = "../../../src-tauri/src/ffmpeg/native_overlay.rs"]
mod overlay_stream;
#[cfg(feature = "current")]
#[allow(dead_code)]
#[path = "../../../src-tauri/src/ffmpeg/runner.rs"]
mod runner;
#[cfg(feature = "current")]
#[allow(dead_code)]
mod stubs;
#[cfg(feature = "current")]
use stubs::{danmu2ass, progress};

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FPS: f64 = 60.0;
const FRAMES: u64 = 360;
const FRAME_BYTES: usize = WIDTH as usize * HEIGHT as usize * 4;
// Boundary, moving, fully populated and blank frames in both waves. Equality
// checking is separate from timed runs so hashing does not distort CPU timings.
const SAMPLES: [u64; 14] = [
    0, 1, 30, 90, 167, 168, 179, 180, 181, 210, 270, 347, 348, 359,
];

fn scene(assets: &Path, name: &str) -> Result<RenderInput, String> {
    let unique = match name {
        "repeated" | "mixed" => 4,
        "unique" => 100,
        _ => return Err(format!("Unknown scene: {name}")),
    };
    let images = (0..100)
        .map(|id| {
            let start = if id < 50 { 0.0 } else { 3.0 };
            let slot = id % 50;
            let x = 80.0 + (slot % 10) as f32 * 170.0;
            let y = 100.0 + (slot / 10) as f32 * 170.0;
            ImageEvent {
                path: assets.join(format!("image-{:03}.png", id % unique)),
                start,
                end: start + 2.8,
                from: (x, y),
                to: (x - 60.0, y + 40.0),
                width: 96.0,
                height: 96.0,
                opacity: 0.8,
            }
        })
        .collect();
    let mut documents = vec![];
    if name == "mixed" {
        let mut text = String::from("[Script Info]\nScriptType: v4.00+\nPlayResX: 1920\nPlayResY: 1080\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,Arial,36,&H33FFFFFF,&H00FFFFFF,&H33000000,&H33000000,0,0,0,0,100,100,0,0,1,2,1,7,20,20,20,1\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n");
        for i in 0..24 {
            let x = 200 + (i / 12) * 800;
            let y = 20 + (i % 12) * 80;
            text.push_str(&format!("Dialogue: 0,0:00:00.00,0:00:06.00,Default,,0,0,0,,{{\\move({x},{y},{},{y})}}中文 Hello {i} 😀👩🏽‍💻👍🏽\n", x-150));
        }
        documents.push(AssSource {
            text,
            kind: SourceKind::Danmaku,
        });
    }
    Ok(RenderInput { documents, images })
}

#[cfg(feature = "current")]
fn render_stats(s: &native_overlay::RenderStats) -> Value {
    json!({
        "frames": s.frames,
        "text_layout_ms": s.text_layout.as_secs_f64() * 1000.0,
        "text_raster_ms": s.text_raster.as_secs_f64() * 1000.0,
        "image_prepare_ms": s.image_prepare.as_secs_f64() * 1000.0,
        "composition_ms": s.composition.as_secs_f64() * 1000.0,
        "readback_ms": s.readback.as_secs_f64() * 1000.0,
        "text_cache_hits": s.text_cache_hits, "text_cache_misses": s.text_cache_misses,
        "image_cache_hits": s.image_cache_hits, "image_cache_misses": s.image_cache_misses,
    })
}

fn core(assets: &Path, name: &str, check: bool, profile: bool) -> Result<Value, String> {
    // Fixture construction and process launch are excluded from measurements.
    let input = scene(assets, name)?;
    let init = Instant::now();
    let mut renderer = Renderer::new(WIDTH, HEIGHT, input, vec![])?;
    let initialization = init.elapsed();
    #[cfg(feature = "current")]
    renderer.set_profiling(profile);
    #[cfg(not(feature = "current"))]
    if profile {
        return Err("Baseline has no stage profiling API".into());
    }
    #[cfg(feature = "current")]
    let mut bytes = vec![0; FRAME_BYTES];
    let started = Instant::now();
    let mut hashes = Vec::new();
    let indices: Vec<u64> = if check {
        SAMPLES.to_vec()
    } else {
        (0..FRAMES).collect()
    };
    for frame in indices {
        #[cfg(feature = "current")]
        renderer.frame_into(frame as f64 / FPS, &mut bytes)?;
        #[cfg(not(feature = "current"))]
        let bytes = renderer.frame(frame as f64 / FPS)?;
        if bytes.len() != FRAME_BYTES {
            return Err("Unexpected frame size".into());
        }
        if check {
            hashes.push(json!({"frame": frame, "sha256": format!("{:x}", Sha256::digest(&bytes))}));
        }
        black_box(&bytes);
    }
    let elapsed = started.elapsed();
    let mut result = json!({
        "mode": if check { "pixel_check" } else { "core" }, "scene": name,
        "implementation": if cfg!(feature = "current") { "current" } else { "baseline" },
        "frames": if check { SAMPLES.len() as u64 } else { FRAMES },
        "initialization_ms": initialization.as_secs_f64() * 1000.0,
        "render_loop_ms": elapsed.as_secs_f64() * 1000.0,
        "profiled": profile,
    });
    if check {
        result["samples"] = json!(hashes);
    }
    #[cfg(feature = "current")]
    if profile {
        result["render"] = render_stats(&renderer.stats());
    }
    Ok(result)
}

#[cfg(feature = "current")]
async fn stream(
    assets: &Path,
    name: &str,
    ffmpeg: &str,
    output: &str,
    encoder_profile: bool,
) -> Result<Value, String> {
    use overlay_stream::{add_overlay_input, run_overlay_command_profiled, OverlaySpec};
    let spec = OverlaySpec {
        input: scene(assets, name)?,
        width: WIDTH,
        height: HEIGHT,
        fps: FPS,
        start: 0.0,
        duration: FRAMES as f64 / FPS,
    };
    let mut command = tokio::process::Command::new(ffmpeg);
    command.args([
        "-hide_banner",
        "-nostdin",
        "-y",
        "-filter_complex_threads",
        "1",
        "-f",
        "lavfi",
        "-i",
        "color=c=0x203040:s=1920x1080:r=60:d=6",
    ]);
    if encoder_profile {
        command.arg("-benchmark_all");
    }
    add_overlay_input(&mut command, &spec).await?;
    command.args([
        "-filter_complex",
        "[0:v]format=bgra[base];[base][1:v]overlay=shortest=1:format=rgb:alpha=premultiplied,format=yuv420p[v]",
        "-map",
        "[v]",
        "-an",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "18",
        "-threads",
        "2",
        "-frames:v",
        "360",
        output,
    ]);
    let args: Vec<String> = command
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let (job, s) = run_overlay_command_profiled::<stubs::NoReporter>(
        command,
        spec,
        runner::ProgressMode::None,
        "overlay-perf",
        None,
    )
    .await?;
    if s.feed.frames != FRAMES || s.feed.bytes != FRAMES * FRAME_BYTES as u64 {
        return Err(format!(
            "Incomplete feed: {} frames, {} bytes",
            s.feed.frames, s.feed.bytes
        ));
    }
    if s.buffer_allocations != 3 {
        return Err(format!(
            "Expected 3 recycled buffers, got {}",
            s.buffer_allocations
        ));
    }
    Ok(json!({
        "mode": "production_stream", "scene": name, "frames": s.feed.frames,
        "ffmpeg_args": args, "buffer_allocations": s.buffer_allocations,
        "initialization_ms": s.initialization.as_secs_f64() * 1000.0,
        "buffer_allocation_ms": s.buffer_allocation.as_secs_f64() * 1000.0,
        "buffer_wait_ms": s.buffer_wait.as_secs_f64() * 1000.0,
        "queue_send_wait_ms": s.queue_send_wait.as_secs_f64() * 1000.0,
        "queue_receive_wait_ms": s.feed.queue_wait.as_secs_f64() * 1000.0,
        "pipe_write_ms": s.feed.pipe_write.as_secs_f64() * 1000.0,
        "bytes": s.feed.bytes, "wall_ms": s.wall.as_secs_f64() * 1000.0,
        "ffmpeg_drain_ms": s.ffmpeg_drain.map(|d| d.as_secs_f64() * 1000.0),
        "render": render_stats(&s.render),
        "ffmpeg_encoder_calls": if encoder_profile { Some(encoder_benchmark(&job.logs)) } else { None },
    }))
}

/// FFmpeg -benchmark_all intervals, not pure encoder CPU or total wall time.
#[cfg(feature = "current")]
fn encoder_benchmark(logs: &[String]) -> Value {
    let mut count = 0u64;
    let (mut user, mut sys, mut real) = (0u64, 0u64, 0u64);
    for line in logs {
        let fields: Vec<_> = line.split_whitespace().collect();
        let Some(start) = fields.iter().position(|s| *s == "bench:") else {
            continue;
        };
        let f = &fields[start..];
        if f.len() < 8 || !matches!(f[7], "encode_video" | "flush_video") {
            continue;
        }
        if let (Ok(u), Ok(s), Ok(r)) = (
            f[1].parse::<u64>(),
            f[3].parse::<u64>(),
            f[5].parse::<u64>(),
        ) {
            count += 1;
            user += u;
            sys += s;
            real += r;
        }
    }
    json!({"calls":count,"reported_user_ms":user as f64/1000.0,"reported_sys_ms":sys as f64/1000.0,"reported_real_ms":real as f64/1000.0})
}

fn run() -> Result<Value, String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        return Err("Usage: overlay-perf MODE ASSETS SCENE [FFMPEG OUTPUT]".into());
    }
    let assets = Path::new(&args[2]);
    match args[1].as_str() {
        "core" => core(assets, &args[3], false, false),
        "profile" => core(assets, &args[3], false, true),
        "check" => core(assets, &args[3], true, false),
        #[cfg(feature = "current")]
        "stream" | "encoder-profile" if args.len() == 6 => {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?
                .block_on(stream(
                    assets,
                    &args[3],
                    &args[4],
                    &args[5],
                    args[1] == "encoder-profile",
                ))
        }
        _ => Err("Invalid mode or arguments".into()),
    }
}

fn main() {
    match run() {
        Ok(result) => println!("{result}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
