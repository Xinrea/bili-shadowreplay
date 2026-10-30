//! Matched render/composite/encode benchmark. There is no source-video decoder
//! in the current Vulkan PoC, so these numbers are not whole-project exports.
mod cpu;
mod gpu;
mod scene;

use serde_json::json;
use std::path::PathBuf;

pub const BACKGROUND: [u8; 4] = scene::BACKGROUND;

#[derive(Clone, Debug)]
pub struct BenchmarkSpec {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub frames: u32,
    pub bit_rate: u64,
    pub gop: u32,
}

fn run() -> Result<(), String> {
    let mut mode = String::new();
    let mut workload = "normal".to_owned();
    let mut frames = 1800;
    let mut output = PathBuf::from("benchmark.mp4");
    let mut args = std::env::args().skip(1);
    while let Some(name) = args.next() {
        if name == "--help" {
            println!("vulkan-benchmark --mode cpu|gpu [--scene normal|dense|poc] [--frames 1800] [--output FILE]");
            return Ok(());
        }
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {name}"))?;
        match name.as_str() {
            "--mode" => mode = value,
            "--scene" => workload = value,
            "--frames" => frames = value.parse::<u32>().map_err(|e| e.to_string())?,
            "--output" => output = value.into(),
            _ => return Err(format!("Unknown benchmark option {name}")),
        }
    }
    if !matches!(mode.as_str(), "cpu" | "gpu")
        || !matches!(workload.as_str(), "normal" | "dense" | "poc")
        || !(120..=3600).contains(&frames)
    {
        return Err("Require --mode cpu|gpu, --scene normal|dense|poc and 120..3600 frames".into());
    }
    // RenderSettings::default(), hwaccel::project_quality_args and the default
    // 1080p/60 danmaku path. Backend-specific speed controls are recorded in the
    // result, rather than pretending Vulkan quality 0 is NVENC's p1 preset.
    let spec = BenchmarkSpec {
        width: 1920,
        height: 1080,
        fps: 60,
        frames,
        bit_rate: 10_000_000,
        gop: 120,
    };
    let assets = tempfile::tempdir().map_err(|e| e.to_string())?;
    let red = assets.path().join("red.png");
    std::fs::write(&red, include_bytes!("../../assets/red.png")).map_err(|e| e.to_string())?;
    let input = scene::make(&workload, f64::from(frames) / f64::from(spec.fps), &red);
    let stats = match mode.as_str() {
        "cpu" => cpu::run(&spec, input, &output)?,
        "gpu" => gpu::run(&spec, input, &output)?,
        _ => unreachable!(),
    };
    println!(
        "{}",
        json!({
            "mode": mode, "scene": workload, "output": output,
            "scope": "same-scene render/composite/convert/encode subpipeline; no source-video decode/audio/project cache",
            "project_defaults": {"codec":"hevc", "priority":"speed", "bit_rate":spec.bit_rate,
                "gop":spec.gop,"width":spec.width,"height":spec.height,"fps":spec.fps},
            "frames":frames,"video_seconds":f64::from(frames)/f64::from(spec.fps),
            "scene_settings": {"normal_messages_per_second":2,"dense_messages_per_second":10,
                "flight_seconds":8,"font_size_ass_720":36,"opacity":0.8},
            "stats": stats
        })
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Vulkan benchmark: {error}");
        std::process::exit(1);
    }
}
