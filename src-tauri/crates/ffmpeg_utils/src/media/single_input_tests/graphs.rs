//! Draft for the shared native graph executor; helpers live in its parent
//! single_input_tests module. CLI comparisons remain strict packet oracles.
use super::*;

fn video_options() -> VideoEncodeOptions {
    VideoEncodeOptions {
        threads: Some(1),
        gop: Some(120),
        options: [
            ("preset".into(), "medium".into()),
            ("crf".into(), "20".into()),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    }
}
fn binding(input: usize, video: bool) -> GraphInput {
    GraphInput {
        input,
        selector: if video {
            StreamSelector::FirstVideo
        } else {
            StreamSelector::FirstAudio
        },
        label: format!("{input}:{}", if video { "v" } else { "a" }),
        required: true,
    }
}
fn graph_route(label: &str, output: usize, mode: StreamMode) -> StreamRoute {
    StreamRoute {
        source: StreamSource::Filter {
            label: label.into(),
        },
        output,
        required: true,
        mode,
    }
}
fn graph_plan(
    inputs: Vec<MediaInput>,
    outputs: Vec<MediaOutput>,
    spec: &str,
    bindings: Vec<GraphInput>,
    routes: Vec<StreamRoute>,
    graph_outputs: Vec<GraphOutput>,
) -> MediaPlan {
    MediaPlan {
        inputs,
        outputs,
        routes,
        filter_graph: Some(FilterGraphPlan {
            spec: spec.into(),
            inputs: bindings,
            outputs: graph_outputs,
        }),
        ..Default::default()
    }
}
fn output(label: &str, video: bool) -> GraphOutput {
    GraphOutput {
        label: label.into(),
        kind: if video {
            MediaKind::Video
        } else {
            MediaKind::Audio
        },
    }
}

#[tokio::test]
async fn native_decoded_concat_and_crossfade_match_cli_packets_and_timing() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.mp4");
    synthetic(&first);
    let second = directory.path().join("second.mp4");
    for mixed_fps in [false, true] {
        let input_filter = if mixed_fps {
            "hue=h=90,fps=24"
        } else {
            "hue=h=90"
        };
        cli(
            &strings(&[
                "-i",
                first.to_str().unwrap(),
                "-vf",
                input_filter,
                "-af",
                "volume=0.75",
                "-c:v",
                "libx264",
                "-threads:v",
                "1",
                "-c:a",
                "aac",
            ]),
            &second,
        );
        for transitions in [false, true] {
            let filter = if transitions && mixed_fps {
                "[0:v]fps=12,settb=AVTB[v0];[1:v]fps=12,settb=AVTB[v1];[v0][v1]xfade=transition=fade:duration=1:offset=2[v];[0:a][1:a]acrossfade=d=1[a]"
            } else if transitions {
                "[0:v]settb=AVTB[v0];[1:v]settb=AVTB[v1];[v0][v1]xfade=transition=fade:duration=1:offset=2[v];[0:a][1:a]acrossfade=d=1[a]"
            } else {
                "[0:v][0:a][1:v][1:a]concat=n=2:v=1:a=1[v][a]"
            };
            let target = directory.path().join("native.mp4");
            let oracle = directory.path().join("oracle.mp4");
            let plan = graph_plan(
                vec![MediaInput::file(&first), MediaInput::file(&second)],
                vec![MediaOutput::file(&target)],
                filter,
                vec![
                    binding(0, true),
                    binding(0, false),
                    binding(1, true),
                    binding(1, false),
                ],
                vec![
                    graph_route("v", 0, StreamMode::Video(video_options())),
                    graph_route("a", 0, StreamMode::Audio(AudioEncodeOptions::default())),
                ],
                vec![output("v", true), output("a", false)],
            );
            execute_media_plan(plan, None).await.unwrap();
            cli(
                &strings(&[
                    "-i",
                    first.to_str().unwrap(),
                    "-i",
                    second.to_str().unwrap(),
                    "-filter_complex",
                    filter,
                    "-map",
                    "[v]",
                    "-map",
                    "[a]",
                    "-c:v",
                    "libx264",
                    "-preset",
                    "medium",
                    "-crf",
                    "20",
                    "-g",
                    "120",
                    "-threads:v",
                    "1",
                    "-c:a",
                    "aac",
                ]),
                &oracle,
            );
            assert_movie(
                &snapshot(&target),
                &snapshot(&oracle),
                &format!("multi-input transition={transitions} mixed-fps={mixed_fps}"),
            );
        }
    }
    assert_clean(directory.path());
}

#[derive(Default)]
struct Samples(Vec<i16>);
impl crate::AudioSink for Samples {
    fn write(&mut self, samples: &[i16]) -> Result<(), String> {
        self.0.extend_from_slice(samples);
        Ok(())
    }
}
async fn pcm(path: &Path) -> Vec<i16> {
    crate::decode_audio(
        path,
        crate::AudioDecodeOptions { sample_rate: 48000 },
        Samples::default(),
    )
    .await
    .unwrap()
    .sink
    .0
}
fn split_plan(source: &Path, video: &Path, audio: &Path) -> MediaPlan {
    let audio_options = AudioEncodeOptions {
        encoder: "pcm_s16le".into(),
        sample_rate: Some(48000),
        channel_layout: Some("mono".into()),
        ..Default::default()
    };
    graph_plan(
        vec![MediaInput::file(source)],
        vec![MediaOutput::file(video), MediaOutput::file(audio)],
        "[0:v]scale=64:48[v];[0:a]anull[a]",
        vec![binding(0, true), binding(0, false)],
        vec![
            graph_route("v", 0, StreamMode::Video(video_options())),
            graph_route("a", 1, StreamMode::Audio(audio_options)),
        ],
        vec![output("v", true), output("a", false)],
    )
}

#[tokio::test]
async fn native_graph_writes_video_and_pcm_outputs_before_completion_without_cli() {
    let directory = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video/subtitle-black.mp4");
    let video = directory.path().join("video.mp4");
    let audio = directory.path().join("audio.wav");
    let expected = pcm(&source).await;
    let targets = [video.clone(), audio.clone()];
    let result = execute_media_plan(
        split_plan(&source, &video, &audio),
        Some(Box::new(move |progress| {
            if progress.stage == MediaStage::Completed {
                for path in &targets {
                    assert!(fs::metadata(path).unwrap().len() > 0);
                }
            }
        })),
    )
    .await
    .unwrap();
    assert_eq!(result.outputs, [video.clone(), audio.clone()]);
    let movie = snapshot(&video);
    assert_eq!(movie.streams.len(), 1);
    assert_eq!(movie.streams[0].kind, media::Type::Video);
    assert_eq!(movie.streams[0].size, (64, 48));
    assert_eq!(movie.streams[0].packets.len(), 11);
    assert_eq!(pcm(&audio).await, expected);
    assert_clean(directory.path());
}

#[tokio::test]
async fn invalid_second_graph_output_keeps_all_existing_outputs() {
    let directory = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video/subtitle-black.mp4");
    let video = directory.path().join("kept.mp4");
    fs::write(&video, b"previous video").unwrap();
    let bad = directory.path().join("directory.wav");
    fs::create_dir(&bad).unwrap();
    assert!(execute_media_plan(split_plan(&source, &video, &bad), None)
        .await
        .is_err());
    assert_eq!(fs::read(&video).unwrap(), b"previous video");
    assert!(bad.is_dir());
    assert_clean(directory.path());
}

#[cfg(unix)]
#[tokio::test]
async fn late_graph_output_failure_does_not_publish_another_file() {
    let directory = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video/subtitle-black.mp4");
    let video = directory.path().join("kept.mp4");
    fs::write(&video, b"previous video").unwrap();
    let other = directory.path().join("other");
    fs::create_dir(&other).unwrap();
    let audio = other.join("audio.wav");
    let removed = other.clone();
    let error = execute_media_plan(
        split_plan(&source, &video, &audio),
        Some(Box::new(move |update| {
            if update.stage == MediaStage::Finalizing {
                fs::remove_dir_all(&removed).unwrap();
            }
        })),
    )
    .await;
    assert!(error.is_err());
    assert_eq!(fs::read(&video).unwrap(), b"previous video");
    assert_clean(directory.path());
}

#[tokio::test]
async fn native_generated_video_source_is_finite_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("red.mp4");
    let mut source = MediaInput::generated_video(ColorSource {
        color: "red".into(),
        width: 64,
        height: 48,
        frame_rate_num: 10,
        frame_rate_den: 1,
    });
    source.duration = Some(0.5);
    let plan = graph_plan(
        vec![source],
        vec![MediaOutput::file(&target)],
        "[0:v]null[v]",
        vec![binding(0, true)],
        vec![graph_route("v", 0, StreamMode::Video(video_options()))],
        vec![output("v", true)],
    );
    let result = execute_media_plan(plan, None).await.unwrap();
    assert_eq!(result.video_frames, 5);
    let movie = snapshot(&target);
    assert_eq!(movie.streams.len(), 1);
    assert_eq!(movie.streams[0].size, (64, 48));
    assert_eq!(movie.streams[0].packets.len(), 5);
    assert_clean(directory.path());
}

#[tokio::test]
async fn graph_resource_lease_remains_alive_until_after_output_publication() {
    use std::sync::atomic::Ordering;
    struct Workspace {
        directory: tempfile::TempDir,
        alive: Arc<AtomicBool>,
    }
    impl Drop for Workspace {
        fn drop(&mut self) {
            assert!(self.directory.path().is_dir());
            self.alive.store(false, Ordering::Release);
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_path_buf();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video/subtitle-black.mp4");
    let video = path.join("video.mp4");
    let audio = path.join("audio.wav");
    let alive = Arc::new(AtomicBool::new(true));
    let mut request = split_plan(&source, &video, &audio);
    request.resources.push(MediaResourceLease::new(Workspace {
        directory,
        alive: alive.clone(),
    }));
    let observed = alive.clone();
    let completed = Arc::new(AtomicBool::new(false));
    let finished = completed.clone();
    execute_media_plan(
        request,
        Some(Box::new(move |progress| {
            if progress.stage == MediaStage::Completed {
                assert!(
                    observed.load(Ordering::Acquire),
                    "workspace released before publication"
                );
                assert!(fs::metadata(&video).unwrap().len() > 0);
                assert!(fs::metadata(&audio).unwrap().len() > 0);
                finished.store(true, Ordering::Release);
            }
        })),
    )
    .await
    .unwrap();
    assert!(completed.load(Ordering::Acquire));
    assert!(!alive.load(Ordering::Acquire));
    assert!(
        !path.exists(),
        "workspace cleanup did not run after publication"
    );
}

#[tokio::test]
async fn all_audio_mapping_keeps_each_track_order_metadata_and_packets() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original.mp4");
    synthetic(&original);
    let source = directory.path().join("dual-audio.mp4");
    cli(
        &strings(&[
            "-i",
            original.to_str().unwrap(),
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=900:sample_rate=44100:duration=3",
            "-map",
            "0:v",
            "-map",
            "0:a",
            "-map",
            "1:a",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-metadata:s:a:0",
            "language=eng",
            "-metadata:s:a:1",
            "language=jpn",
            "-disposition:a:0",
            "0",
            "-disposition:a:1",
            "default",
        ]),
        &source,
    );
    assert_eq!(
        snapshot(&source)
            .streams
            .iter()
            .filter(|stream| stream.kind == media::Type::Audio)
            .count(),
        2
    );
    for graph in [false, true] {
        let target = directory.path().join("native.mp4");
        let oracle = directory.path().join("oracle.mp4");
        let all_audio = StreamRoute {
            source: StreamSource::Input {
                input: 0,
                selector: StreamSelector::AllAudio,
            },
            output: 0,
            required: false,
            mode: StreamMode::Copy,
        };
        let (request, args) = if graph {
            let filter = "[0:v]scale=64:48[v]";
            (
                graph_plan(
                    vec![MediaInput::file(&source)],
                    vec![MediaOutput::file(&target)],
                    filter,
                    vec![binding(0, true)],
                    vec![
                        graph_route("v", 0, StreamMode::Video(video_options())),
                        all_audio,
                    ],
                    vec![output("v", true)],
                ),
                strings(&[
                    "-i",
                    source.to_str().unwrap(),
                    "-filter_complex",
                    filter,
                    "-map",
                    "[v]",
                    "-map",
                    "0:a?",
                    "-c:v",
                    "libx264",
                    "-preset",
                    "medium",
                    "-crf",
                    "20",
                    "-g",
                    "120",
                    "-threads:v",
                    "1",
                    "-c:a",
                    "copy",
                ]),
            )
        } else {
            (
                MediaPlan {
                    inputs: vec![MediaInput::file(&source)],
                    outputs: vec![MediaOutput::file(&target)],
                    routes: vec![
                        StreamRoute {
                            source: StreamSource::Input {
                                input: 0,
                                selector: StreamSelector::FirstVideo,
                            },
                            output: 0,
                            required: true,
                            mode: StreamMode::Copy,
                        },
                        all_audio,
                    ],
                    ..Default::default()
                },
                strings(&[
                    "-i",
                    source.to_str().unwrap(),
                    "-map",
                    "0:v:0",
                    "-map",
                    "0:a?",
                    "-c",
                    "copy",
                ]),
            )
        };
        execute_media_plan(request, None).await.unwrap();
        cli(&args, &oracle);
        let native = snapshot(&target);
        assert_eq!(
            native
                .streams
                .iter()
                .filter(|stream| stream.kind == media::Type::Audio)
                .count(),
            2
        );
        assert_movie(
            &native,
            &snapshot(&oracle),
            &format!("all-audio graph={graph}"),
        );
    }
    assert_clean(directory.path());
}

#[tokio::test]
async fn global_graph_explicit_cfr_30_preserves_cli_duplicate_frames() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source.mp4");
    synthetic(&source);
    let target = directory.path().join("native.mp4");
    let oracle = directory.path().join("oracle.mp4");
    let mut video = video_options();
    video.frame_rate = FrameRatePolicy::Constant { num: 30, den: 1 };
    let request = graph_plan(
        vec![MediaInput::file(&source)],
        vec![MediaOutput::file(&target)],
        "[0:v]null[v]",
        vec![binding(0, true)],
        vec![graph_route("v", 0, StreamMode::Video(video))],
        vec![output("v", true)],
    );
    execute_media_plan(request, None).await.unwrap();
    cli(
        &strings(&[
            "-i",
            source.to_str().unwrap(),
            "-filter_complex",
            "[0:v]null[v]",
            "-map",
            "[v]",
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "20",
            "-g",
            "120",
            "-threads:v",
            "1",
            "-r",
            "30",
        ]),
        &oracle,
    );
    assert_movie(
        &snapshot(&target),
        &snapshot(&oracle),
        "shared graph explicit CFR 30",
    );
    assert_clean(directory.path());
}

#[tokio::test]
async fn empty_audio_apad_gets_continuous_pts_and_accepts_a_terminal_semicolon() {
    let directory = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video/subtitle-black.mp4");
    let target = directory.path().join("silence.wav");
    let spec = "[0:a]atrim=end_sample=0,apad=whole_len=4800[a];\n";
    let options = AudioEncodeOptions {
        encoder: "pcm_s16le".into(),
        sample_rate: Some(48000),
        channel_layout: Some("mono".into()),
        ..Default::default()
    };
    let request = graph_plan(
        vec![MediaInput::file(&source)],
        vec![MediaOutput::file(&target)],
        spec,
        vec![binding(0, false)],
        vec![graph_route("a", 0, StreamMode::Audio(options))],
        vec![output("a", false)],
    );
    execute_media_plan(request, None).await.unwrap();
    let samples = pcm(&target).await;
    assert_eq!(samples.len(), 4800);
    assert!(samples.iter().all(|sample| *sample == 0));
    if has_cli() {
        let oracle = directory.path().join("oracle.wav");
        cli(
            &strings(&[
                "-i",
                source.to_str().unwrap(),
                "-filter_complex",
                spec,
                "-map",
                "[a]",
                "-c:a",
                "pcm_s16le",
                "-ar",
                "48000",
                "-ac",
                "1",
            ]),
            &oracle,
        );
        assert_movie(
            &snapshot(&target),
            &snapshot(&oracle),
            "empty apad and terminal semicolon",
        );
    }
    assert_clean(directory.path());
}
