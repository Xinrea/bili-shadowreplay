use super::*;
use crate::*;
use ffmpeg_next::{self as ffmpeg, ffi, format, frame, Packet};
use std::{collections::VecDeque, path::Path, rc::Rc, sync::atomic::Ordering};

struct Delayed {
    template: VideoFrame,
    frames: VecDeque<VideoFrame>,
    fail: bool,
    seen: Arc<std::sync::Mutex<Vec<i64>>>,
    // This deliberately non-Send object must be created on the media worker.
    _thread_local: Rc<()>,
}
impl VideoFrameProcessor for Delayed {
    fn output_template(&self) -> Result<VideoFrame, String> {
        Ok(self.template.clone())
    }
    fn process(&mut self, mut frame: VideoFrame) -> Result<Vec<VideoFrame>, String> {
        if self.fail {
            return Err("processor deliberately failed".into());
        }
        let pts = frame.pts().unwrap();
        self.seen.lock().unwrap().push(pts);
        let result = unsafe { ffi::av_frame_make_writable(frame.as_mut_ptr()) };
        assert!(result >= 0);
        frame.data_mut(0)[0] = 20 + pts as u8;
        self.frames.push_back(frame);
        Ok(if self.frames.len() > 2 {
            vec![self.frames.pop_front().unwrap()]
        } else {
            Vec::new()
        })
    }
    fn flush(&mut self) -> Result<Vec<VideoFrame>, String> {
        Ok(self.frames.drain(..).collect())
    }
}

fn plan(path: &Path, start: f64, duration: f64) -> MediaPlan {
    let mut video = MediaInput::generated_video(ColorSource {
        color: "blue".into(),
        width: 32,
        height: 24,
        frame_rate_num: 25,
        frame_rate_den: 1,
    });
    video.duration = Some(0.24);
    let mut audio = MediaInput::generated_audio(SilenceSource {
        sample_rate: 48_000,
        channel_layout: "mono".into(),
    });
    audio.duration = Some(0.24);
    let mut output = MediaOutput::file(path);
    output.format = Some("matroska".into());
    output.options.insert("allow_raw_vfw".into(), "1".into());
    output.seek = Some(start);
    output.duration = Some(duration);
    MediaPlan {
        inputs: vec![video, audio],
        outputs: vec![output],
        filter_graph: Some(FilterGraphPlan {
            spec: "[video]null[base];[audio]anull[sound]".into(),
            inputs: vec![
                GraphInput {
                    input: 0,
                    selector: StreamSelector::AutoVideo,
                    label: "video".into(),
                    required: true,
                },
                GraphInput {
                    input: 1,
                    selector: StreamSelector::AutoAudio,
                    label: "audio".into(),
                    required: true,
                },
            ],
            outputs: vec![
                GraphOutput {
                    label: "base".into(),
                    kind: MediaKind::Video,
                },
                GraphOutput {
                    label: "sound".into(),
                    kind: MediaKind::Audio,
                },
            ],
        }),
        routes: vec![
            StreamRoute {
                source: StreamSource::Filter {
                    label: "base".into(),
                },
                output: 0,
                required: true,
                mode: StreamMode::Video(VideoEncodeOptions {
                    encoder: "rawvideo".into(),
                    frame_processor: true,
                    frame_rate: FrameRatePolicy::Constant { num: 25, den: 1 },
                    ..Default::default()
                }),
            },
            StreamRoute {
                source: StreamSource::Filter {
                    label: "sound".into(),
                },
                output: 0,
                required: true,
                mode: StreamMode::Audio(AudioEncodeOptions {
                    encoder: "pcm_s16le".into(),
                    ..Default::default()
                }),
            },
        ],
        ..Default::default()
    }
}

fn factory(
    fail: bool,
    seen: Arc<std::sync::Mutex<Vec<i64>>>,
) -> Box<dyn VideoFrameProcessorFactory> {
    Box::new(move |info: VideoFrameInfo, cancelled: Arc<AtomicBool>| {
        assert_eq!(info.time_base, (1, 25).into());
        assert_eq!(info.frame_rate, (25, 1).into());
        assert!(!cancelled.load(Ordering::Acquire));
        let mut template = frame::Video::empty();
        template.set_format(format::Pixel::NV12);
        template.set_width(info.width);
        template.set_height(info.height);
        unsafe {
            (*template.as_mut_ptr()).sample_aspect_ratio = ffi::AVRational { num: 1, den: 1 };
        }
        Ok(Box::new(Delayed {
            template,
            frames: VecDeque::new(),
            fail,
            seen,
            _thread_local: Rc::new(()),
        }) as Box<dyn VideoFrameProcessor>)
    })
}

#[tokio::test]
async fn delayed_video_processor_preserves_trimmed_timestamps_audio_and_drain_without_cli() {
    for (start, duration, count) in [(0.0, 0.24, 6), (0.08, 0.12, 3)] {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("processed.mkv");
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let result = execute_with_sources(
            plan(&output, start, duration),
            MediaBindings {
                video_processors: vec![(0, factory(false, seen.clone()))],
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.video_frames, count);
        assert_eq!(result.audio_samples, (duration * 48_000.0).round() as u64);
        assert_eq!(*seen.lock().unwrap(), (0..count as i64).collect::<Vec<_>>());
        let mut input =
            crate::metadata::open_input(&output, Arc::new(AtomicBool::new(false))).unwrap();
        let mut frames = 0;
        let mut sound = 0;
        loop {
            let mut packet = Packet::empty();
            match packet.read(&mut input) {
                Ok(()) => {
                    let stream = input.stream(packet.stream()).unwrap();
                    if stream.parameters().medium() == ffmpeg::media::Type::Video {
                        packet.rescale_ts(stream.time_base(), (1, 25));
                        assert_eq!(packet.pts(), Some(frames));
                        assert_eq!(packet.data().unwrap()[0], 20 + frames as u8);
                        frames += 1;
                    } else {
                        sound += packet.size();
                    }
                }
                Err(ffmpeg::Error::Eof) => break,
                Err(error) => panic!("demux failed: {error}"),
            }
        }
        assert_eq!(frames as u64, count);
        assert_eq!(sound as u64, result.audio_samples * 2);
    }
}

#[tokio::test]
async fn processor_binding_and_worker_errors_preserve_existing_outputs() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("keep.mkv");
    std::fs::write(&output, b"previous recording").unwrap();
    let error = execute_media_plan(plan(&output, 0.0, 0.24), None)
        .await
        .unwrap_err();
    assert!(error.message.contains("缺少绑定"), "{error}");
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let error = execute_with_sources(
        plan(&output, 0.0, 0.24),
        MediaBindings {
            video_processors: vec![(0, factory(true, seen))],
            ..Default::default()
        },
        None,
    )
    .await
    .unwrap_err();
    assert!(
        error.message.contains("processor deliberately failed"),
        "{error}"
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"previous recording");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}
