//! Complete real-video export through the production compositor and MediaPlan.
//! Scene preparation is shared with the comparison worker; no app/speech/V8
//! runtime or replacement rendering/encoding algorithm is linked here.
mod ffmpeg;
#[allow(dead_code)]
mod stubs;
use stubs::{danmu2ass, progress, project};

use ffmpeg_utils::{
    FilterGraphPlan, FrameRatePolicy, GraphInput, GraphOutput, MediaInput, MediaKind, MediaOutput,
    MediaPlan, StreamMode, StreamRoute, StreamSelector, StreamSource,
};
use native_overlay::{AssSource, ImageEvent, RenderInput, SourceKind};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

fn number(value: &Value, key: &str) -> Result<f64, String> {
    value[key]
        .as_f64()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("Missing finite number: {key}"))
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("Missing string: {key}"))
}
fn point(value: &Value, key: &str) -> Result<(f32, f32), String> {
    Ok((
        number(&value[key], "x")? as f32,
        number(&value[key], "y")? as f32,
    ))
}
fn scene(value: &Value, root: &Path) -> Result<RenderInput, String> {
    let documents = value["documents"]
        .as_array()
        .ok_or("Missing documents")?
        .iter()
        .map(|item| {
            Ok(AssSource {
                text: std::fs::read_to_string(root.join(text(item, "path")?))
                    .map_err(|e| e.to_string())?,
                kind: match text(item, "kind")? {
                    "danmaku" => SourceKind::Danmaku,
                    "subtitle" => SourceKind::Subtitle,
                    other => return Err(format!("Unknown ASS source kind: {other}")),
                },
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let images = value["overlays"]
        .as_array()
        .ok_or("Missing overlays")?
        .iter()
        .map(|item| {
            Ok(ImageEvent {
                path: root.join(text(item, "path")?),
                start: number(item, "start")?,
                end: number(item, "end")?,
                from: point(item, "from")?,
                to: point(item, "to")?,
                width: number(item, "width")? as f32,
                height: number(item, "height")? as f32,
                opacity: number(item, "opacity")? as f32,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(RenderInput { documents, images })
}

// Exact graph builder from src-tauri/src/ffmpeg/danmu.rs. Kept here to avoid
// linking unrelated recorder/network/download dependencies into this runner.
fn overlay_graph(fps: f64, pipeline: ffmpeg::hwaccel::RenderPipeline) -> String {
    use ffmpeg::hwaccel;
    if pipeline.uses_gpu() {
        return format!(
            "[0:v]{},fps={fps},scale=out_color_matrix=bt709:out_range=tv,format=nv12[outv]\n",
            hwaccel::H264_SCALE_PAD_FILTER,
        );
    }
    let mut filter = format!(
        "[0:v]{},fps={fps},format=bgra[base];\n\
         [base][1:v]overlay=shortest=1:format=rgb:alpha=premultiplied[composited];\n",
        hwaccel::H264_SCALE_PAD_FILTER,
    );
    let format = hwaccel::output_format_filter(pipeline.encoder);
    filter.push_str(&format!("[composited]{format}[outv]\n"));
    filter
}

// Exact media-plan construction from the production danmaku entry point.
fn overlay_media_plan(
    input: PathBuf,
    output: PathBuf,
    graph: String,
    label: &str,
    video: ffmpeg_utils::VideoEncodeOptions,
) -> MediaPlan {
    let gpu_composition = video.frame_processor;
    MediaPlan {
        inputs: vec![MediaInput::file(input)],
        outputs: vec![MediaOutput::file(output)],
        routes: vec![
            StreamRoute {
                source: StreamSource::Filter {
                    label: label.into(),
                },
                output: 0,
                required: true,
                mode: StreamMode::Video(video),
            },
            StreamRoute {
                source: StreamSource::Input {
                    input: 0,
                    selector: StreamSelector::AllAudio,
                },
                output: 0,
                required: false,
                mode: StreamMode::Copy,
            },
        ],
        filter_graph: Some(FilterGraphPlan {
            spec: graph,
            inputs: if gpu_composition {
                vec![GraphInput {
                    input: 0,
                    selector: StreamSelector::FirstVideo,
                    label: "0:v".into(),
                    required: true,
                }]
            } else {
                vec![
                    GraphInput {
                        input: 0,
                        selector: StreamSelector::FirstVideo,
                        label: "0:v".into(),
                        required: true,
                    },
                    GraphInput {
                        input: 1,
                        selector: StreamSelector::FirstVideo,
                        label: "1:v".into(),
                        required: true,
                    },
                ]
            },
            outputs: vec![GraphOutput {
                label: label.into(),
                kind: MediaKind::Video,
            }],
        }),
        ..Default::default()
    }
}

async fn run(
    scenario_path: &Path,
    output: &Path,
    require_gpu: bool,
    measure_stages: bool,
    decoder_threads: Option<usize>,
    blocking_handoff: bool,
) -> Result<Value, String> {
    let wall = Instant::now();
    let scenario_path = scenario_path.canonicalize().map_err(|e| e.to_string())?;
    let root = scenario_path.parent().ok_or("Scenario has no parent")?;
    let bytes = std::fs::read(&scenario_path).map_err(|e| e.to_string())?;
    let scenario: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let source = root
        .join(text(&scenario, "source")?)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let output = std::path::absolute(output).map_err(|e| e.to_string())?;
    let settings = project::RenderSettings::default();
    if text(&scenario, "codec")? != "hevc"
        || number(&scenario, "bitrate_bps")? != 10_000_000.0
        || number(&scenario, "width")? != 1920.0
        || number(&scenario, "height")? != 1080.0
    {
        return Err("Scenario must match the production HEVC/Speed/10Mbps/1080p defaults".into());
    }
    let input = scene(&scenario, root)?;
    let metadata = ffmpeg_utils::extract_video_metadata(&source).await?;
    if metadata.audio_codec.is_empty() {
        return Err("The complete comparison source must include audio".into());
    }
    let fps = metadata.fps.max(60.0);
    let duration = number(&scenario, "duration_seconds")?;
    if fps != number(&scenario, "fps")? || (metadata.duration - duration).abs() > 0.1 {
        return Err("Scenario duration/FPS does not match the complete source video".into());
    }
    let probe = Instant::now();
    let mut pipeline = ffmpeg::hwaccel::get_render_pipeline(&settings).await;
    if pipeline.uses_gpu() && !ffmpeg_utils::video_is_sdr_bt709(&source).await? {
        pipeline =
            ffmpeg::hwaccel::RenderPipeline::cpu(ffmpeg::get_cpu_render_encoder(&settings).await);
    }
    if require_gpu && !pipeline.uses_gpu() {
        return Err("GPU export was required but capability/source probing selected CPU".into());
    }
    let probe_ms = probe.elapsed().as_secs_f64() * 1000.0;
    let graph = overlay_graph(fps, pipeline);
    let mut video = pipeline.video_options(None, None, Some(&settings))?;
    video.filter = None;
    let (num, den) = ffmpeg_utils::parse_frame_rate(fps).map_err(|error| error.to_string())?;
    video.frame_rate = FrameRatePolicy::Constant { num, den };
    let profile = json!({"encoder": pipeline.encoder, "renderer": pipeline.cache_identity(), "gop": video.gop,
        "bit_rate": video.bit_rate, "options": video.options, "frame_processor": video.frame_processor});
    eprintln!("current_export_profile={profile}");
    let mut media =
        overlay_media_plan(source.clone(), output.clone(), graph.clone(), "outv", video);
    media.inputs[0].decoder_threads = decoder_threads;
    let started = Instant::now();
    let profiler = measure_stages.then(ffmpeg_utils::profiling::MediaProfiler::default);
    let spec = ffmpeg::native_overlay::OverlaySpec {
        input,
        width: 1920,
        height: 1080,
        fps,
        start: number(&scenario, "start")?,
        duration,
    };
    let result = if blocking_handoff {
        if !pipeline.uses_gpu() {
            return Err("Handoff comparison requires the GPU graph".into());
        }
        ffmpeg::native_gpu::run_overlay_plan_with_handoff(
            media,
            spec,
            ffmpeg_utils::vulkan_video::EncoderFrameHandoff::Blocking,
            profiler.clone(),
            None::<&stubs::Silent>,
        )
        .await?
    } else if let Some(profiler) = &profiler {
        if !pipeline.uses_gpu() {
            return Err("Stage profiling requires the GPU graph".into());
        }
        ffmpeg::native_gpu::run_overlay_plan_measured(
            media,
            spec,
            profiler.clone(),
            None::<&stubs::Silent>,
        )
        .await?
    } else {
        ffmpeg::native_overlay::run_overlay_plan(
            media,
            spec,
            1,
            ffmpeg::runner::ProgressMode::None,
            "Full export comparison",
            None::<&stubs::Silent>,
        )
        .await?
    };
    let export_ms = started.elapsed().as_secs_f64() * 1000.0;
    let expected_frames = (duration * fps).round() as u64;
    if result.video_frames != expected_frames {
        return Err(format!(
            "Incomplete video: {} frames, expected {expected_frames}",
            result.video_frames
        ));
    }
    let wall_ms = wall.elapsed().as_secs_f64() * 1000.0;
    use sha2::{Digest, Sha256};
    Ok(
        json!({"implementation": "current-production", "scenario": scenario_path,
        "scenario_sha256": format!("{:x}", Sha256::digest(&bytes)),
        "source": source, "output": output, "platform": std::env::consts::OS,
        "linked_ffmpeg_version": ffmpeg_utils::linked_ffmpeg_version(),
        "stage_profile": profiler.map(|p| p.report()), "decoder_threads_override": decoder_threads,
        "blocking_handoff": blocking_handoff,
        "profile": profile, "filter_graph": graph, "width": 1920, "height": 1080,
        "fps": fps, "duration_seconds": duration, "frames": result.video_frames,
        "audio_mode": "copy", "source_audio_codec": metadata.audio_codec,
        "audio_samples": result.audio_samples, "counts": scenario["counts"],
        "encoder_probe_ms": probe_ms, "export_ms": export_ms, "wall_ms": wall_ms,
        "encoded_fps": result.video_frames as f64 / (export_ms / 1000.0)}),
    )
}

/// An untimed, transparent CPU reference for validating the finished videos.
/// This is a separate command and never runs inside the export measurement.
fn render_reference(scenario_path: &Path, time: f64, output: &Path) -> Result<Value, String> {
    if !time.is_finite() || time < 0.0 {
        return Err("Reference time must be finite and non-negative".into());
    }
    let scenario_path = scenario_path
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let bytes = std::fs::read(&scenario_path).map_err(|error| error.to_string())?;
    let scenario: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let root = scenario_path.parent().ok_or("Scenario has no parent")?;
    if number(&scenario, "width")? != 1920.0 || number(&scenario, "height")? != 1080.0 {
        return Err("Reference must match the production 1080p scenario".into());
    }
    let render_time = time + number(&scenario, "start")?;
    if !render_time.is_finite() {
        return Err("Reference timeline is outside the supported range".into());
    }
    let mut renderer =
        native_overlay::Renderer::new(1920, 1080, scene(&scenario, root)?, Vec::new())?;
    let png = renderer.png(render_time)?;
    let output = std::path::absolute(output).map_err(|error| error.to_string())?;
    std::fs::write(&output, &png).map_err(|error| error.to_string())?;
    use sha2::{Digest, Sha256};
    Ok(json!({
        "mode": "render-reference", "scenario": scenario_path,
        "scenario_sha256": format!("{:x}", Sha256::digest(&bytes)),
        "time_seconds": time, "render_time_seconds": render_time,
        "output": output, "png_sha256": format!("{:x}", Sha256::digest(&png)),
        "missingGlyphs": renderer.missing_glyph_count(), "width": 1920, "height": 1080,
    }))
}

#[tokio::main]
async fn main() {
    struct Logger;
    impl log::Log for Logger {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() <= log::Level::Info
        }
        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                eprintln!("{}: {}", record.level(), record.args());
            }
        }
        fn flush(&self) {}
    }
    if std::env::var_os("RUST_LOG").is_some() {
        let _ = log::set_logger(&Logger);
        log::set_max_level(log::LevelFilter::Info);
    }
    let mut scenario = None;
    let mut output = None;
    let mut reference = false;
    let mut require_gpu = false;
    let mut measure_stages = false;
    let mut decoder_threads = None;
    let mut blocking_handoff = false;
    let mut time = None;
    let mut args = std::env::args_os().skip(1);
    let result = (|| -> Result<(), String> {
        while let Some(arg) = args.next() {
            if arg == "--blocking-handoff" {
                if blocking_handoff {
                    return Err("Duplicate handoff option".into());
                }
                blocking_handoff = true;
                continue;
            }
            if arg == "--decoder-threads" {
                if decoder_threads.is_some() {
                    return Err("Duplicate decoder thread option".into());
                }
                let value = args.next().ok_or("Missing decoder thread count")?;
                let value = value
                    .to_str()
                    .ok_or("Invalid decoder thread count")?
                    .parse::<usize>()
                    .map_err(|error| error.to_string())?;
                if value == 0 || value > i32::MAX as usize {
                    return Err("Decoder threads must be positive".into());
                }
                decoder_threads = Some(value);
                continue;
            }
            if arg == "--measure-stages" {
                measure_stages = true;
                continue;
            }
            if arg == "--require-gpu" {
                require_gpu = true;
                continue;
            }
            if arg == "--render-reference" {
                if reference {
                    return Err("Duplicate reference option".into());
                }
                reference = true;
                continue;
            }
            if arg == "--time" {
                if time.is_some() {
                    return Err("Duplicate reference time".into());
                }
                time = Some(
                    args.next()
                        .ok_or("Missing reference time")?
                        .to_str()
                        .ok_or("Invalid reference time")?
                        .parse::<f64>()
                        .map_err(|error| error.to_string())?,
                );
                continue;
            }
            let slot = match arg.to_str() {
                Some("--scenario") => &mut scenario,
                Some("--output") => &mut output,
                _ => {
                    return Err(
                        "Usage: current-export-comparison [--require-gpu] [--measure-stages] [--decoder-threads N] [--blocking-handoff] [--render-reference --time SECONDS] --scenario PATH --output PATH".into(),
                    )
                }
            };
            if slot.is_some() {
                return Err("Duplicate CLI option".into());
            }
            *slot = Some(PathBuf::from(
                args.next().ok_or("Missing CLI option value")?,
            ));
        }
        Ok(())
    })();
    let result = match (result, scenario, output) {
        (Ok(()), Some(scenario), Some(output)) => match (reference, time) {
            (true, Some(time)) => render_reference(&scenario, time, &output),
            (false, None) => {
                run(
                    &scenario,
                    &output,
                    require_gpu,
                    measure_stages,
                    decoder_threads,
                    blocking_handoff,
                )
                .await
            }
            _ => Err("--render-reference and --time must be used together".into()),
        },
        (Err(error), _, _) => Err(error),
        _ => Err("--scenario and --output are required".into()),
    };
    match result {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("Export comparison: {error}");
            std::process::exit(1);
        }
    }
}
