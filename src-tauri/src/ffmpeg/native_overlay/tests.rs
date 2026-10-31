use super::*;
use ffmpeg_utils::{
    ColorSource, FilterGraphPlan, FrameRatePolicy, GraphInput, GraphOutput, MediaKind, MediaOutput,
    StreamMode, StreamRoute, StreamSelector, StreamSource, VideoEncodeOptions,
};
use std::{path::Path, process::Command};

#[derive(Clone)]
struct Reporter;
#[async_trait::async_trait]
impl ProgressReporterTrait for Reporter {
    async fn update(&self, _: &str) {}
    async fn finish(&self, _: bool, _: &str) {}
}
fn spec() -> OverlaySpec {
    OverlaySpec {
        input: native_overlay::RenderInput::default(),
        width: 64,
        height: 64,
        fps: 30.0,
        start: 0.0,
        duration: 1.0,
    }
}
fn plan(spec: &OverlaySpec, output: &Path, background: [u8; 3]) -> MediaPlan {
    let (num, den) = ffmpeg_utils::parse_frame_rate(spec.fps).unwrap();
    let mut base = MediaInput::generated_video(ColorSource {
        color: format!(
            "0x{:02X}{:02X}{:02X}",
            background[0], background[1], background[2]
        ),
        width: spec.width,
        height: spec.height,
        frame_rate_num: num,
        frame_rate_den: den,
    });
    base.duration = Some(1.0);
    let mut output = MediaOutput::file(output);
    output.format = Some("rawvideo".into());
    MediaPlan {
        inputs: vec![base],
        outputs: vec![output],
        routes: vec![StreamRoute {
            source: StreamSource::Filter { label: "v".into() },
            output: 0,
            required: true,
            mode: StreamMode::Video(VideoEncodeOptions {
                encoder: "rawvideo".into(),
                threads: Some(1),
                frame_rate: FrameRatePolicy::Constant { num, den },
                ..Default::default()
            }),
        }],
        filter_graph: Some(FilterGraphPlan {
            spec: "[0:v]format=bgra[base];[base][1:v]overlay=shortest=1:format=rgb:alpha=premultiplied,format=bgra[v]".into(),
            inputs: (0..2).map(|input| GraphInput {
                input,
                selector: StreamSelector::FirstVideo,
                label: format!("{input}:v"),
                required: true,
            }).collect(),
            outputs: vec![GraphOutput { label: "v".into(), kind: MediaKind::Video }],
        }),
        ..Default::default()
    }
}
#[test]
fn raw_input_is_premultiplied_bgra_and_rounds_up_partial_frames() {
    let mut spec = spec();
    spec.width = 1920;
    spec.height = 1080;
    spec.fps = 60.0;
    spec.duration = 1.01;
    assert_eq!(spec.layout().unwrap(), (1920 * 1080 * 4, 61));
    let mut plan = MediaPlan::default();
    let raw = append_input(&mut plan, &spec, 0).unwrap();
    assert_eq!(raw.alpha, RawVideoAlpha::Premultiplied);
    assert_eq!(raw.frame_count, 61);
    assert_eq!((raw.frame_rate_num, raw.frame_rate_den), (60, 1));
    assert!(append_input(&mut plan, &spec, 0).is_err());
}
#[test]
fn rejects_unbounded_work_before_appending_input() {
    for invalid in [f64::NAN, f64::INFINITY, -1.0, 0.0, 241.0] {
        let mut spec = spec();
        spec.fps = invalid;
        let mut plan = MediaPlan::default();
        assert!(append_input(&mut plan, &spec, 0).is_err());
        assert!(plan.inputs.is_empty());
    }
    let mut spec = spec();
    spec.width = u32::MAX;
    assert!(spec.layout().is_err());
    spec.width = 1920;
    spec.duration = f64::MAX;
    assert!(spec.layout().is_err());
    spec.duration = 1.0;
    spec.start = f64::NAN;
    assert!(spec.layout().is_err());
}
#[tokio::test]
async fn dropping_guard_closes_a_backpressured_source_without_a_worker() {
    let (sender, input) = ffmpeg_utils::raw_video_channel(
        RawVideoSpec::bgra(1, 1, 30.0, 2, RawVideoAlpha::Premultiplied).unwrap(),
        1,
    )
    .unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    let stop = StopRendering {
        cancelled: flag.clone(),
        source: input.cancel_handle(),
    };
    sender
        .blocking_send(Ok(RawVideoFrame::Frame(vec![0; 4])))
        .unwrap();
    let (ready, wait) = tokio::sync::oneshot::channel();
    let worker = tokio::task::spawn_blocking(move || {
        ready.send(()).unwrap();
        assert!(sender
            .blocking_send(Ok(RawVideoFrame::Frame(vec![0; 4])))
            .is_err());
        assert!(flag.load(Ordering::Acquire));
    });
    wait.await.unwrap();
    drop(stop);
    tokio::time::timeout(Duration::from_secs(1), worker)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn pooled_native_stream_profiles_real_frames_and_stops_at_primary_eof() {
    let directory = tempfile::tempdir().unwrap();
    for early in [false, true] {
        let mut spec = spec();
        spec.duration = if early { 60.0 } else { 1.0 };
        let output = directory.path().join("pooled.bgra");
        let plan = plan(&spec, &output, [0, 0, 0]);
        let (_, stats) = tokio::time::timeout(
            Duration::from_secs(15),
            run_overlay_plan_profiled::<Reporter>(
                plan,
                spec,
                1,
                ProgressMode::None,
                "pooled native test",
                None,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(std::fs::metadata(output).unwrap().len(), 30 * 64 * 64 * 4);
        assert_eq!(stats.buffer_allocations, FRAME_BUFFERS);
        assert_eq!(stats.feed.bytes, stats.feed.frames * 64 * 64 * 4);
        if early {
            assert!(stats.feed.frames < 1800);
        } else {
            assert_eq!(stats.feed.frames, 30);
            assert_eq!(stats.render.frames, 30);
            assert!(stats.native_drain.is_some());
        }
    }
}
fn visual_input() -> native_overlay::RenderInput {
    use native_overlay::{AssSource, ImageEvent, RenderInput, SourceKind};
    RenderInput {
        documents: vec![AssSource {
            kind: SourceKind::Subtitle,
            text: "[Script Info]\nScriptType: v4.00+\nPlayResX: 128\nPlayResY: 96\n\
                [V4+ Styles]\n\
                Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
                Style: Default,Arial,28,&H600040E0,&H00FFFFFF,&H8000C040,&H90000000,0,0,0,0,100,100,0,0,1,2,3,5,0,0,0,1\n\
                [Events]\n\
                Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
                Dialogue: 0,0:00:00.00,0:00:00.80,Default,,0,0,0,,Bg\n".into(),
        }],
        images: vec![ImageEvent {
            path: Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ffmpeg/native_overlay/tests/translucent.png"),
            start: 0.2, end: 0.8, from: (0.0, 0.0), to: (0.0, 0.0),
            width: 480.0, height: 360.0, opacity: 0.5,
        }],
    }
}
#[tokio::test]
async fn premultiplied_native_stream_matches_straight_preview_on_coloured_backgrounds() {
    let directory = tempfile::tempdir().unwrap();
    let input = visual_input();
    let (width, height, fps, frames) = (128, 96, 10.0, 10usize);
    let frame_bytes = width as usize * height as usize * 4;
    let mut preview = system_renderer(width, height, input.clone()).unwrap();
    let previews: Vec<_> = (0..frames)
        .map(|frame| preview.frame(frame as f64 / fps).unwrap())
        .collect();
    assert!(previews[5].chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
    assert!(previews[9].chunks_exact(4).all(|p| p == [0, 0, 0, 0]));
    for background in [[0u8, 0, 0], [255, 255, 255], [16, 192, 224]] {
        let output = directory.path().join("composited.bgra");
        let spec = OverlaySpec {
            input: input.clone(),
            width,
            height,
            fps,
            start: 0.0,
            duration: 1.0,
        };
        run_overlay_plan::<Reporter>(
            plan(&spec, &output, background),
            spec,
            1,
            ProgressMode::None,
            "premultiplied native test",
            None,
        )
        .await
        .unwrap();
        let actual = std::fs::read(&output).unwrap();
        assert_eq!(actual.len(), frame_bytes * frames);
        for (frame, (drawn, composited)) in previews
            .iter()
            .zip(actual.chunks_exact(frame_bytes))
            .enumerate()
        {
            for (pixel, (rgba, bgra)) in drawn
                .chunks_exact(4)
                .zip(composited.chunks_exact(4))
                .enumerate()
            {
                assert_eq!(bgra[3], 255);
                for channel in 0..3 {
                    let alpha = u32::from(rgba[3]);
                    let expected = (u32::from(rgba[channel]) * alpha
                        + u32::from(background[channel]) * (255 - alpha)
                        + 127)
                        / 255;
                    let delta = (i32::from(bgra[2 - channel]) - expected as i32).abs();
                    assert!(delta <= 2, "background={background:?} frame={frame} pixel={pixel} channel={channel}: {rgba:?} -> {bgra:?}, expected {expected}");
                }
            }
        }
    }
}
#[tokio::test]
async fn native_alpha_pixels_match_real_cli_exactly() {
    if !Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = visual_input();
    let mut renderer = system_renderer(128, 96, input.clone()).unwrap();
    let mut bytes = vec![];
    for frame in 0..10 {
        let mut pixels = vec![0; 128 * 96 * 4];
        renderer
            .frame_premultiplied_bgra_into(frame as f64 / 10.0, &mut pixels)
            .unwrap();
        bytes.extend(pixels);
    }
    let source = directory.path().join("overlay.bgra");
    std::fs::write(&source, bytes).unwrap();
    for background in [[0u8, 0, 0], [255, 255, 255], [16, 192, 224]] {
        let native = directory.path().join("native.bgra");
        let oracle = directory.path().join("oracle.bgra");
        let spec = OverlaySpec {
            input: input.clone(),
            width: 128,
            height: 96,
            fps: 10.0,
            start: 0.0,
            duration: 1.0,
        };
        run_overlay_plan::<Reporter>(
            plan(&spec, &native, background),
            spec,
            1,
            ProgressMode::None,
            "alpha oracle",
            None,
        )
        .await
        .unwrap();
        let mut cli = Command::new("ffmpeg");
        cli.args([
            "-v",
            "error",
            "-nostdin",
            "-filter_complex_threads",
            "1",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!(
            "color=c=0x{:02X}{:02X}{:02X}:s=128x96:r=10:d=1,format=bgra",
            background[0], background[1], background[2]
        ))
        .args([
            "-f",
            "rawvideo",
            "-pixel_format",
            "bgra",
            "-video_size",
            "128x96",
            "-framerate",
            "10",
        ]);
        if ffmpeg_utils::linked_ffmpeg_version()
            .split('.')
            .next()
            .unwrap()
            .parse::<u32>()
            .unwrap()
            >= 9
        {
            cli.args(["-alpha_mode", "premultiplied"]);
        }
        let result = cli.arg("-i").arg(source.clone()).args(["-filter_complex", "[0:v]format=bgra[base];[base][1:v]overlay=shortest=1:format=rgb:alpha=premultiplied,format=bgra[v]", "-map", "[v]", "-c:v", "rawvideo", "-threads:v", "1", "-r", "10", "-f", "rawvideo", "-y"]).arg(&oracle).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let actual = std::fs::read(native).unwrap();
        let expected = std::fs::read(oracle).unwrap();
        assert_eq!(actual.len(), expected.len());
        let mismatch = actual.iter().zip(&expected).position(|(a, b)| a != b);
        assert!(
            mismatch.is_none(),
            "background={background:?} first differing byte={mismatch:?}"
        );
    }
}
#[test]
fn image_coordinates_and_sizes_share_the_scheduler_scale() {
    let events = image_events(
        &[ImageDanmu {
            image: crate::danmu2ass::DanmuImage {
                path: "emoji.png".into(),
                width: 54.0,
                height: 36.0,
            },
            start: 1.0,
            end: 9.0,
            from: 1280.0,
            to: -54.0,
            top: 36.0,
        }],
        0.8,
    );
    let event = &events[0];
    assert_eq!(event.from, (1920.0, 54.0));
    assert_eq!(event.to, (-81.0, 54.0));
    assert_eq!((event.width, event.height), (81.0, 54.0));
    assert_eq!((event.start, event.end), (1.0, 9.0));
    assert_eq!(event.opacity, 0.8);
}
