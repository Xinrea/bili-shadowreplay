//! Frame-threaded decoding can buffer an entire short clip until EOF. Exercise
//! both executors against the single-thread baseline, including input/output trims.
use super::*;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video/subtitle-black.mp4")
}

#[test]
fn video_decoder_automatic_threads_preserve_explicit_limits_and_audio_defaults() {
    let path = fixture();
    let context = crate::metadata::open_input(&path, Arc::new(AtomicBool::new(false))).unwrap();
    let source = MediaInput::file(&path);
    let video = context.streams().best(media::Type::Video).unwrap();
    assert!(unsafe { (*video.parameters().as_ptr()).video_delay } > 0);
    let open = |input: &MediaInput| {
        util::open_decoder(video.parameters(), video.time_base(), input).unwrap()
    };
    let automatic = open(&source);
    let codec = ffmpeg::decoder::find(video.parameters().id()).unwrap();
    if unsafe { ffmpeg::ffi::av_cpu_count() } > 1
        && codec
            .capabilities()
            .contains(ffmpeg::codec::capabilities::Capabilities::FRAME_THREADS)
    {
        assert!(unsafe { (*automatic.as_ptr()).thread_count } > 1);
        assert_ne!(unsafe { (*automatic.as_ptr()).active_thread_type }, 0);
    }
    for threads in [1, 2] {
        let mut input = source.clone();
        input.decoder_threads = Some(threads);
        assert_eq!(
            unsafe { (*open(&input).as_ptr()).thread_count },
            threads as i32
        );
    }
    let mut options = source.clone();
    options.decoder_options.insert("threads".into(), "1".into());
    assert_eq!(unsafe { (*open(&options).as_ptr()).thread_count }, 1);
    for threads in [0, i32::MAX as usize + 1] {
        let mut input = source.clone();
        input.decoder_threads = Some(threads);
        assert!(util::open_decoder(video.parameters(), video.time_base(), &input).is_err());
    }
    let audio = context.streams().best(media::Type::Audio).unwrap();
    let decoder = util::open_decoder(audio.parameters(), audio.time_base(), &source).unwrap();
    assert_eq!(unsafe { (*decoder.as_ptr()).thread_count }, 1);
}

#[tokio::test]
async fn parallel_video_decode_preserves_b_frames_trims_audio_and_eof_without_cli() {
    let directory = tempfile::tempdir().unwrap();
    for graph in [false, true] {
        // This 11-frame B-frame clip is shorter than the automatic decoder's
        // buffering depth on our 12/24-core hosts. Tail frames need EOF drain.
        for (input_seek, output_seek, duration, frames) in [
            (None, None, None, 11),
            (None, Some(0.3), Some(0.4), 4),
            (Some(0.3), None, Some(0.4), 4),
        ] {
            let make = |output: &Path, threads| {
                let mut request = plan(
                    &fixture(),
                    output,
                    StreamMode::Video(VideoEncodeOptions {
                        threads: Some(1),
                        gop: Some(120),
                        options: [
                            ("preset".into(), "medium".into()),
                            ("crf".into(), "20".into()),
                        ]
                        .into(),
                        ..Default::default()
                    }),
                    StreamMode::Copy,
                );
                request.inputs[0].decoder_threads = threads;
                request.inputs[0].seek = input_seek.map(|seconds| InputSeek {
                    seconds,
                    accurate: true,
                });
                request.outputs[0].seek = output_seek;
                request.outputs[0].duration = duration;
                if graph {
                    request.routes[0].source = StreamSource::Filter { label: "v".into() };
                    request.filter_graph = Some(FilterGraphPlan {
                        spec: "[0:v]null[v]".into(),
                        inputs: vec![GraphInput {
                            input: 0,
                            selector: StreamSelector::FirstVideo,
                            label: "0:v".into(),
                            required: true,
                        }],
                        outputs: vec![GraphOutput {
                            label: "v".into(),
                            kind: MediaKind::Video,
                        }],
                    });
                }
                request
            };
            let single = directory.path().join("single.mp4");
            let automatic = directory.path().join("automatic.mp4");
            for (output, threads) in [(&single, Some(1)), (&automatic, None)] {
                let result = execute_media_plan(make(output, threads), None)
                    .await
                    .unwrap();
                assert_eq!(
                    result.video_frames, frames,
                    "graph={graph}, input={input_seek:?}, output={output_seek:?}"
                );
            }
            assert_movie(
                &snapshot(&automatic),
                &snapshot(&single),
                &format!("auto decode graph={graph}, input={input_seek:?}, output={output_seek:?}"),
            );
            assert_clean(directory.path());
        }
    }
}

#[tokio::test]
async fn parallel_video_decode_drains_high_resolution_tail_with_bounded_pending_frames() {
    let directory = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video/test.mp4");
    let output = directory.path().join("downscaled.mp4");
    // 3240x2160 H.264. A frame-threaded EOF burst exceeds the graph's 64 MiB
    // pending-frame budget even though the output is small; drain incrementally.
    let mut request = plan(
        &source,
        &output,
        StreamMode::Video(VideoEncodeOptions {
            threads: Some(1),
            options: [("preset".into(), "ultrafast".into())].into(),
            ..Default::default()
        }),
        StreamMode::Copy,
    );
    request.routes[0].source = StreamSource::Filter { label: "v".into() };
    request.filter_graph = Some(FilterGraphPlan {
        spec: "[0:v]scale=64:48[v]".into(),
        inputs: vec![GraphInput {
            input: 0,
            selector: StreamSelector::FirstVideo,
            label: "0:v".into(),
            required: true,
        }],
        outputs: vec![GraphOutput {
            label: "v".into(),
            kind: MediaKind::Video,
        }],
    });
    let result = execute_media_plan(request, None).await.unwrap();
    assert_eq!(result.video_frames, 327);
    let movie = snapshot(&output);
    assert_eq!(movie.streams[0].packets.len(), 327);
    assert_eq!(movie.streams[0].size, (64, 48));
    assert!(movie.streams[0]
        .packets
        .windows(2)
        .all(|p| p[0].pts < p[1].pts));
    assert_clean(directory.path());
}
