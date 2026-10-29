use super::*;

use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    process::{Command, Stdio},
    sync::{atomic::AtomicBool, Arc, Mutex},
};

use ffmpeg_next::{self as ffmpeg, media, Packet};

#[derive(Debug, PartialEq, Eq)]
struct CodedPacket {
    payload: Vec<u8>,
    pts: Option<i64>,
    dts: Option<i64>,
    duration: i64,
    key: bool,
    side_data: Vec<(String, Vec<u8>)>,
}

#[derive(Debug, PartialEq, Eq)]
struct Stream {
    kind: media::Type,
    disposition: i32,
    codec: ffmpeg::codec::Id,
    size: (i32, i32),
    audio: (i32, i32),
    time_base: (i32, i32),
    rate: (i32, i32),
    start: i64,
    duration: i64,
    extradata: Vec<u8>,
    metadata: BTreeMap<String, String>,
    side_data: Vec<(String, Vec<u8>)>,
    packets: Vec<CodedPacket>,
}

struct Movie {
    metadata: BTreeMap<String, String>,
    streams: Vec<Stream>,
}

fn snapshot(path: &Path) -> Movie {
    let mut input = crate::metadata::open_input(path, Arc::new(AtomicBool::new(false))).unwrap();
    let mut result = Movie {
        metadata: input
            .metadata()
            .iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect(),
        streams: input
            .streams()
            .map(|stream| {
                let parameters = stream.parameters();
                let raw = unsafe { &*parameters.as_ptr() };
                Stream {
                    kind: parameters.medium(),
                    disposition: unsafe { (*stream.as_ptr()).disposition },
                    codec: parameters.id(),
                    size: (raw.width, raw.height),
                    audio: (raw.sample_rate, raw.ch_layout.nb_channels),
                    time_base: (
                        stream.time_base().numerator(),
                        stream.time_base().denominator(),
                    ),
                    rate: (
                        stream.avg_frame_rate().numerator(),
                        stream.avg_frame_rate().denominator(),
                    ),
                    start: stream.start_time(),
                    duration: stream.duration(),
                    extradata: if raw.extradata_size == 0 {
                        Vec::new()
                    } else {
                        unsafe {
                            std::slice::from_raw_parts(raw.extradata, raw.extradata_size as usize)
                        }
                        .to_vec()
                    },
                    metadata: stream
                        .metadata()
                        .iter()
                        .map(|(key, value)| (key.into(), value.into()))
                        .collect(),
                    side_data: stream
                        .side_data()
                        .map(|data| (format!("{:?}", data.kind()), data.data().to_vec()))
                        .collect(),
                    packets: Vec::new(),
                }
            })
            .collect(),
    };
    loop {
        let mut packet = Packet::empty();
        match packet.read(&mut input) {
            Ok(()) => result.streams[packet.stream()].packets.push(CodedPacket {
                payload: packet.data().unwrap().to_vec(),
                pts: packet.pts(),
                dts: packet.dts(),
                duration: packet.duration(),
                key: packet.is_key(),
                side_data: packet
                    .side_data()
                    .map(|data| (format!("{:?}", data.kind()), data.data().to_vec()))
                    .collect(),
            }),
            Err(ffmpeg::Error::Eof) => break,
            Err(error) => panic!("media demux failed: {error}"),
        }
    }
    result
}

fn assert_movie(actual: &Movie, expected: &Movie, case: &str) {
    assert_eq!(
        actual.metadata, expected.metadata,
        "{case}: global metadata"
    );
    assert_eq!(
        actual.streams.len(),
        expected.streams.len(),
        "{case}: selected streams"
    );
    for (index, (actual, expected)) in actual.streams.iter().zip(&expected.streams).enumerate() {
        assert_eq!(
            actual.disposition, expected.disposition,
            "{case}: stream {index} disposition"
        );
        assert_eq!(
            (actual.kind, actual.codec, actual.size, actual.audio),
            (expected.kind, expected.codec, expected.size, expected.audio),
            "{case}: stream {index} codec"
        );
        if (actual.time_base, actual.rate, actual.start, actual.duration)
            != (
                expected.time_base,
                expected.rate,
                expected.start,
                expected.duration,
            )
        {
            for (packet, (actual, expected)) in actual
                .packets
                .iter()
                .zip(&expected.packets)
                .enumerate()
                .filter(|(_, (actual, expected))| {
                    (actual.pts, actual.dts, actual.duration)
                        != (expected.pts, expected.dts, expected.duration)
                })
                .take(8)
            {
                eprintln!(
                    "{case}: stream {index} packet {packet} timeline actual={:?} expected={:?}",
                    (actual.pts, actual.dts, actual.duration),
                    (expected.pts, expected.dts, expected.duration)
                );
            }
        }
        assert_eq!(
            (actual.time_base, actual.rate, actual.start, actual.duration),
            (
                expected.time_base,
                expected.rate,
                expected.start,
                expected.duration
            ),
            "{case}: stream {index} timeline"
        );
        assert_eq!(
            actual.extradata, expected.extradata,
            "{case}: stream {index} codec header"
        );
        assert_eq!(
            actual.metadata, expected.metadata,
            "{case}: stream {index} metadata"
        );
        assert_eq!(
            actual.side_data, expected.side_data,
            "{case}: stream {index} side data"
        );
        assert_eq!(
            actual.packets.len(),
            expected.packets.len(),
            "{case}: stream {index} packet count"
        );
        for (packet, (actual, expected)) in actual.packets.iter().zip(&expected.packets).enumerate()
        {
            assert!(
                actual == expected,
                "{case}: stream {index} packet {packet} differs: {:?} vs {:?}",
                (
                    actual.pts,
                    actual.dts,
                    actual.duration,
                    &actual.side_data,
                    actual.payload.len()
                ),
                (
                    expected.pts,
                    expected.dts,
                    expected.duration,
                    &expected.side_data,
                    expected.payload.len()
                )
            );
        }
    }
}

fn plan(input: &Path, output: &Path, video: StreamMode, audio: StreamMode) -> MediaPlan {
    MediaPlan {
        inputs: vec![MediaInput::file(input)],
        outputs: vec![MediaOutput::file(output)],
        routes: vec![
            StreamRoute {
                source: StreamSource::Input {
                    input: 0,
                    selector: StreamSelector::AutoVideo,
                },
                output: 0,

                required: false,
                mode: video,
            },
            StreamRoute {
                source: StreamSource::Input {
                    input: 0,
                    selector: StreamSelector::AutoAudio,
                },
                output: 0,

                required: false,
                mode: audio,
            },
        ],
        ..Default::default()
    }
}

fn recorded(path: &Path) {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video");
    let mut data = fs::read(fixtures.join("init.m4s")).unwrap();
    data.extend(fs::read(fixtures.join("segment.m4s")).unwrap());
    fs::write(path, data).unwrap();
}

fn assert_clean(directory: &Path) {
    assert!(fs::read_dir(directory).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with('.')));
}

#[tokio::test]
async fn native_copy_keeps_recorded_payloads_and_reports_completion_after_publish_without_cli() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("recorded.mp4");
    recorded(&input);
    let output = directory.path().join("copied.mp4");
    let progress = Arc::new(Mutex::new(Vec::new()));
    let observed = progress.clone();
    let target = output.clone();
    let result = execute_media_plan(
        plan(&input, &output, StreamMode::Copy, StreamMode::Copy),
        Some(Box::new(move |update| {
            if update.stage == MediaStage::Completed {
                assert!(target.is_file());
                assert!(fs::metadata(&target).unwrap().len() > 0);
            }
            observed.lock().unwrap().push(update);
        })),
    )
    .await
    .unwrap();
    assert_eq!(result.outputs.as_slice(), std::slice::from_ref(&output));
    let source = snapshot(&input);
    let copied = snapshot(&output);
    assert_eq!(source.streams.len(), copied.streams.len());
    for (source, copied) in source.streams.iter().zip(&copied.streams) {
        assert_eq!(source.codec, copied.codec);
        // The recorded fragment's first AAC packet is not marked key. The
        // original CLI's automatic stream-copy skips it (47 -> 46 packets),
        // then preserves every payload from the first key packet onward.
        let source: Vec<_> = source
            .packets
            .iter()
            .skip_while(|packet| !packet.key)
            .collect();
        assert!(!source.is_empty());
        assert_eq!(source.len(), copied.packets.len());
        assert!(source
            .into_iter()
            .zip(&copied.packets)
            .all(|(source, copied)| source.payload == copied.payload));
    }
    let progress = progress.lock().unwrap();
    assert_eq!(progress.last().unwrap().stage, MediaStage::Completed);
    assert!(progress
        .windows(2)
        .all(|pair| pair[0].processed <= pair[1].processed));
    assert_clean(directory.path());
}

#[tokio::test]
async fn native_software_probe_and_small_video_encode_work_without_cli() {
    let directory = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/video/test.mp4");
    let output = directory.path().join("encoded.mp4");
    let video = VideoEncodeOptions {
        filter: Some("scale=64:48".into()),
        threads: Some(1),
        options: [
            ("preset".into(), "ultrafast".into()),
            ("crf".into(), "20".into()),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    probe_video_encoder(video.clone()).await.unwrap();
    let mut request = plan(
        &source,
        &output,
        StreamMode::Video(video),
        StreamMode::Audio(AudioEncodeOptions::default()),
    );
    request.outputs[0].duration = Some(0.1);
    execute_media_plan(request, None).await.unwrap();
    let movie = snapshot(&output);
    assert_eq!(movie.streams.len(), 1);
    assert_eq!(movie.streams[0].codec, ffmpeg::codec::Id::H264);
    assert_eq!(movie.streams[0].size, (64, 48));
    assert!(!movie.streams[0].packets.is_empty());
    assert_clean(directory.path());
}

#[tokio::test]
async fn invalid_native_plans_and_unavailable_hardware_preserve_output_without_cli() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input.mp4");
    recorded(&input);
    let output = directory.path().join("kept.mp4");
    fs::write(&output, b"previous video").unwrap();
    let missing = directory.path().join("missing.mp4");
    let invalid = directory.path().join("invalid.mp4");
    fs::write(&invalid, b"not media").unwrap();
    for source in [&missing, &invalid, &output] {
        assert!(execute_media_plan(
            plan(source, &output, StreamMode::Copy, StreamMode::Copy),
            None
        )
        .await
        .is_err());
        assert_eq!(fs::read(&output).unwrap(), b"previous video");
    }
    let video = VideoEncodeOptions {
        encoder: "h264_vaapi".into(),
        hardware_device: Some(HardwareDevice {
            kind: "vaapi".into(),
            device: Some("/no/such/render/device".into()),
        }),
        ..Default::default()
    };
    assert!(probe_video_encoder(video.clone()).await.is_err());
    assert!(execute_media_plan(
        plan(&input, &output, StreamMode::Video(video), StreamMode::Copy),
        None
    )
    .await
    .is_err());
    assert_eq!(fs::read(&output).unwrap(), b"previous video");
    let invalid_options = VideoEncodeOptions {
        options: [("not_a_video_option".into(), "1".into())]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let error = probe_video_encoder(invalid_options.clone())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not_a_video_option"));
    assert!(execute_media_plan(
        plan(
            &input,
            &output,
            StreamMode::Video(invalid_options),
            StreamMode::Copy
        ),
        None
    )
    .await
    .is_err());
    assert_eq!(fs::read(&output).unwrap(), b"previous video");
    let mut request = plan(&input, &output, StreamMode::Copy, StreamMode::Copy);
    request.outputs[0].duration = Some(f64::NAN);
    assert!(execute_media_plan(request, None).await.is_err());
    assert_eq!(fs::read(&output).unwrap(), b"previous video");
    assert_clean(directory.path());
}

fn has_cli() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn cli(args: &[String], output: &Path) {
    let result = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-y"])
        .args(args)
        .arg(output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).into()).collect()
}

fn synthetic(input: &Path) {
    cli(
        &strings(&[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=128x72:rate=12:duration=3",
            "-f",
            "lavfi",
            "-i",
            "sine=sample_rate=48000:duration=3",
            "-c:v",
            "libx264",
            "-threads:v",
            "1",
            "-g",
            "12",
            "-bf",
            "2",
            "-c:a",
            "aac",
        ]),
        input,
    );
}

#[tokio::test]
async fn native_copy_and_input_seek_trims_match_cli_packet_timing() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.mp4");
    synthetic(&input);
    let output = directory.path().join("native.mp4");
    let oracle = directory.path().join("oracle.mp4");
    for make_zero in [false, true] {
        let mut request = plan(&input, &output, StreamMode::Copy, StreamMode::Copy);
        let mut args = strings(&[
            "-i",
            input.to_str().unwrap(),
            "-c:v",
            "copy",
            "-c:a",
            "copy",
        ]);
        if make_zero {
            request.outputs[0].negative_ts = NegativeTs::MakeZero;
            request.outputs[0].faststart = true;
            args.extend(strings(&[
                "-avoid_negative_ts",
                "make_zero",
                "-movflags",
                "+faststart",
            ]));
        }
        execute_media_plan(request, None).await.unwrap();
        cli(&args, &oracle);
        assert_movie(
            &snapshot(&output),
            &snapshot(&oracle),
            &format!("copy MakeZero={make_zero}"),
        );
    }
    for start in [0.0, 0.111, 1.123, 2.9, 5.0] {
        let mut request = plan(&input, &output, StreamMode::Copy, StreamMode::Copy);
        request.inputs[0].seek = Some(InputSeek {
            seconds: start,
            accurate: true,
        });
        request.outputs[0].duration = Some(0.777);
        execute_media_plan(request, None).await.unwrap();
        cli(
            &strings(&[
                "-ss",
                &start.to_string(),
                "-i",
                input.to_str().unwrap(),
                "-t",
                "0.777",
                "-c",
                "copy",
            ]),
            &oracle,
        );
        assert_movie(
            &snapshot(&output),
            &snapshot(&oracle),
            &format!("input seek copy {start}"),
        );
    }
}

#[tokio::test]
async fn native_software_encode_and_output_seek_match_cli_profiles_and_frames() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.mp4");
    synthetic(&input);
    let output = directory.path().join("native.mp4");
    let oracle = directory.path().join("oracle.mp4");
    for (input_start, start) in [
        (None, None),
        (None, Some(0.0)),
        (None, Some(0.111)),
        (None, Some(1.123)),
        (Some(0.111), None),
    ] {
        let video = VideoEncodeOptions {
            filter: Some("scale=64:48".into()),
            threads: Some(1),
            gop: Some(120),
            options: [
                ("preset".into(), "medium".into()),
                ("crf".into(), "20".into()),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let mut request = plan(
            &input,
            &output,
            StreamMode::Video(video),
            StreamMode::Audio(AudioEncodeOptions::default()),
        );
        request.outputs[0].seek = start;
        request.inputs[0].seek = input_start.map(|seconds| InputSeek {
            seconds,
            accurate: true,
        });
        request.outputs[0].duration = Some(0.777);
        request.outputs[0].negative_ts = NegativeTs::MakeZero;
        let mut args = Vec::new();
        if let Some(start) = input_start {
            args.extend(strings(&["-ss", &start.to_string()]));
        }
        args.extend(strings(&["-i", input.to_str().unwrap()]));
        if let Some(start) = start {
            args.extend(strings(&["-ss", &start.to_string()]));
        }
        args.extend(strings(&[
            "-t",
            "0.777",
            "-vf",
            "scale=64:48",
            "-c:v",
            "libx264",
            "-threads:v",
            "1",
            "-crf",
            "20",
            "-preset",
            "medium",
            "-g",
            "120",
            "-c:a",
            "aac",
            "-avoid_negative_ts",
            "make_zero",
        ]));
        execute_media_plan(request, None).await.unwrap();
        cli(&args, &oracle);
        assert_movie(
            &snapshot(&output),
            &snapshot(&oracle),
            &format!("encode input seek {input_start:?}, output seek {start:?}"),
        );
    }
}
#[tokio::test]
async fn cancelled_native_media_plan_closes_input_and_cleans_worker_output_without_cli() {
    crate::audio::opus_tests::assert_cancelled_media(
        |input, output| async move {
            let request = MediaPlan {
                inputs: vec![MediaInput::file(input)],
                outputs: vec![MediaOutput::file(output)],
                routes: vec![StreamRoute {
                    source: StreamSource::Input {
                        input: 0,
                        selector: StreamSelector::AutoAudio,
                    },
                    output: 0,

                    required: true,
                    mode: StreamMode::Copy,
                }],
                ..Default::default()
            };
            execute_media_plan(request, None)
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        },
        "wav",
        ".media-job-",
    )
    .await;
}

#[tokio::test]
async fn native_copy_trim_maps_subtitles_as_cli_and_rejects_explicit_incompatible_copies() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.mp4");
    synthetic(&input);
    let subtitles = directory.path().join("source.srt");
    fs::write(
        &subtitles,
        "1\n00:00:00,250 --> 00:00:00,750\nA copied subtitle\n",
    )
    .unwrap();
    let source = directory.path().join("subtitles.mkv");
    cli(
        &strings(&[
            "-i",
            input.to_str().unwrap(),
            "-i",
            subtitles.to_str().unwrap(),
            "-map",
            "0:v",
            "-map",
            "0:a",
            "-map",
            "1:0",
            "-c:v",
            "copy",
            "-c:a",
            "copy",
            "-c:s",
            "srt",
        ]),
        &source,
    );
    let output = directory.path().join("native.mkv");
    let oracle = directory.path().join("oracle.mkv");
    let mut request = plan(&source, &output, StreamMode::Copy, StreamMode::Copy);
    request.inputs[0].seek = Some(InputSeek {
        seconds: 0.111,
        accurate: true,
    });
    request.outputs[0].duration = Some(0.777);
    request.routes.push(StreamRoute {
        source: StreamSource::Input {
            input: 0,
            selector: StreamSelector::AutoSubtitle,
        },
        output: 0,

        required: false,
        mode: StreamMode::Copy,
    });
    execute_media_plan(request.clone(), None).await.unwrap();
    cli(
        &strings(&[
            "-ss",
            "0.111",
            "-i",
            source.to_str().unwrap(),
            "-t",
            "0.777",
            "-c",
            "copy",
        ]),
        &oracle,
    );
    assert_movie(
        &snapshot(&output),
        &snapshot(&oracle),
        "trim copies first subtitle",
    );

    let mp4 = directory.path().join("native.mp4");
    request.outputs[0].path = mp4.clone();
    execute_media_plan(request.clone(), None).await.unwrap();
    let mp4_oracle = directory.path().join("oracle.mp4");
    cli(
        &strings(&[
            "-ss",
            "0.111",
            "-i",
            source.to_str().unwrap(),
            "-t",
            "0.777",
            "-c",
            "copy",
        ]),
        &mp4_oracle,
    );
    let mp4 = snapshot(&mp4);
    assert!(mp4
        .streams
        .iter()
        .all(|stream| stream.kind != media::Type::Subtitle));
    assert_movie(
        &mp4,
        &snapshot(&mp4_oracle),
        "MP4 automatic mapping omits subtitles",
    );

    let incompatible = directory.path().join("kept.mp4");
    fs::write(&incompatible, b"previous video").unwrap();
    request.outputs[0].path = incompatible.clone();
    let subtitle = snapshot(&source)
        .streams
        .iter()
        .position(|stream| stream.kind == media::Type::Subtitle)
        .unwrap();
    request.routes.retain(|route| {
        !matches!(
            route.source,
            StreamSource::Input {
                selector: StreamSelector::AutoSubtitle,
                ..
            }
        )
    });
    request.routes.push(StreamRoute {
        source: StreamSource::Input {
            input: 0,
            selector: StreamSelector::Index(subtitle),
        },
        output: 0,

        required: true,
        mode: StreamMode::Copy,
    });
    assert!(execute_media_plan(request, None).await.is_err());
    assert_eq!(fs::read(&incompatible).unwrap(), b"previous video");
    let result = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-y", "-ss", "0.111", "-i"])
        .arg(&source)
        .args([
            "-t", "0.777", "-map", "0:v", "-map", "0:a", "-map", "0:s:0", "-c", "copy",
        ])
        .arg(directory.path().join("explicit.mp4"))
        .output()
        .unwrap();
    assert!(
        !result.status.success(),
        "explicit SRT copy must reject the incompatible MP4 codec"
    );
    assert_clean(directory.path());
}

fn default_subtitle_plan(
    input: &Path,
    output: &Path,
    video: StreamMode,
    audio: StreamMode,
) -> MediaPlan {
    let mut request = plan(input, output, video, audio);
    request.routes.push(StreamRoute {
        source: StreamSource::Input {
            input: 0,
            selector: StreamSelector::AutoSubtitle,
        },
        output: 0,

        required: false,
        mode: StreamMode::Subtitle(SubtitleEncodeOptions::default()),
    });
    request
}

#[tokio::test]
async fn native_subtitle_only_input_encodes_ass_without_cli() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("原生 字幕.srt");
    fs::write(&input, "1\n00:00:00,000 --> 00:00:00,500\n原生字幕\n\n2\n00:00:01,000 --> 00:00:01,500\nSecond cue\n").unwrap();
    let output = directory.path().join("subtitles.mkv");
    execute_media_plan(
        default_subtitle_plan(&input, &output, StreamMode::Copy, StreamMode::Copy),
        None,
    )
    .await
    .unwrap();
    let actual = snapshot(&output);
    assert_eq!(actual.streams.len(), 1);
    let subtitles = &actual.streams[0];
    assert_eq!(subtitles.kind, media::Type::Subtitle);
    assert_eq!(subtitles.codec, ffmpeg::codec::Id::ASS);
    assert_eq!(subtitles.time_base, (1, 1000));
    assert_eq!(subtitles.packets.len(), 2);
    assert_eq!(
        (subtitles.packets[0].pts, subtitles.packets[0].duration),
        (Some(0), 500)
    );
    assert_eq!(
        (subtitles.packets[1].pts, subtitles.packets[1].duration),
        (Some(1000), 500)
    );
    assert!(String::from_utf8_lossy(&subtitles.packets[0].payload).contains("原生字幕"));
    assert!(String::from_utf8_lossy(&subtitles.extradata).contains("[Script Info]"));
    assert_clean(directory.path());
}

#[tokio::test]
async fn native_default_ass_subtitles_match_cli_input_and_output_seek_boundaries() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.mp4");
    synthetic(&input);
    let subtitles = directory.path().join("source.srt");
    fs::write(&subtitles, "1\n00:00:00,250 --> 00:00:00,750\n原生 subtitle\n\n2\n00:00:01,500 --> 00:00:02,500\n<i>A cue across the trim end</i>\n\n3\n00:00:02,800 --> 00:00:03,500\nLast cue\n").unwrap();
    let source = directory.path().join("subtitled.mkv");
    cli(
        &strings(&[
            "-i",
            input.to_str().unwrap(),
            "-i",
            subtitles.to_str().unwrap(),
            "-map",
            "0:v",
            "-map",
            "0:a",
            "-map",
            "1:0",
            "-c:v",
            "copy",
            "-c:a",
            "copy",
            "-c:s",
            "srt",
            "-metadata",
            "title=Subtitled source",
            "-metadata:s:s:0",
            "language=jpn",
        ]),
        &source,
    );
    let output = directory.path().join("native.mkv");
    let oracle = directory.path().join("oracle.mkv");
    for (input_seek, output_seek, duration) in [
        (None, None, None),
        (Some(0.111), None, Some(1.8)),
        (Some(1.123), None, Some(0.777)),
        (None, Some(0.111), Some(1.8)),
    ] {
        let encode = output_seek.is_some();
        let mut request = if encode {
            default_subtitle_plan(
                &source,
                &output,
                StreamMode::Video(VideoEncodeOptions {
                    threads: Some(1),
                    gop: Some(120),
                    options: [
                        ("preset".into(), "medium".into()),
                        ("crf".into(), "20".into()),
                    ]
                    .into_iter()
                    .collect(),
                    ..Default::default()
                }),
                StreamMode::Audio(AudioEncodeOptions::default()),
            )
        } else {
            default_subtitle_plan(&source, &output, StreamMode::Copy, StreamMode::Copy)
        };
        request.inputs[0].seek = input_seek.map(|seconds| InputSeek {
            seconds,
            accurate: true,
        });
        request.outputs[0].seek = output_seek;
        request.outputs[0].duration = duration;
        let mut args = Vec::new();
        if let Some(start) = input_seek {
            args.extend(strings(&["-ss", &start.to_string()]));
        }
        args.extend(strings(&["-i", source.to_str().unwrap()]));
        if let Some(start) = output_seek {
            args.extend(strings(&["-ss", &start.to_string()]));
        }
        if let Some(duration) = duration {
            args.extend(strings(&["-t", &duration.to_string()]));
        }
        if encode {
            args.extend(strings(&[
                "-c:v",
                "libx264",
                "-threads:v",
                "1",
                "-preset",
                "medium",
                "-crf",
                "20",
                "-g",
                "120",
                "-c:a",
                "aac",
                "-avoid_negative_ts",
                "make_zero",
            ]));
            request.outputs[0].negative_ts = NegativeTs::MakeZero;
        } else {
            args.extend(strings(&["-c:v", "copy", "-c:a", "copy"]));
        }
        execute_media_plan(request, None).await.unwrap();
        cli(&args, &oracle);
        assert_movie(
            &snapshot(&output),
            &snapshot(&oracle),
            &format!(
                "default ASS input={input_seek:?} output={output_seek:?} duration={duration:?}"
            ),
        );
    }
    let output = directory.path().join("no-subtitle.mp4");
    let oracle = directory.path().join("no-subtitle-oracle.mp4");
    execute_media_plan(
        default_subtitle_plan(&source, &output, StreamMode::Copy, StreamMode::Copy),
        None,
    )
    .await
    .unwrap();
    cli(
        &strings(&[
            "-i",
            source.to_str().unwrap(),
            "-c:v",
            "copy",
            "-c:a",
            "copy",
        ]),
        &oracle,
    );
    let actual = snapshot(&output);
    assert!(actual
        .streams
        .iter()
        .all(|stream| stream.kind != media::Type::Subtitle));
    assert_movie(
        &actual,
        &snapshot(&oracle),
        "MP4 has no default subtitle encoder",
    );
    assert_clean(directory.path());
}

#[tokio::test]
async fn native_rotation_matrix_copy_and_autorotate_match_cli() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.mp4");
    synthetic(&input);
    let source = directory.path().join("rotated.mp4");
    cli(
        &strings(&[
            "-display_rotation",
            "90",
            "-i",
            input.to_str().unwrap(),
            "-c",
            "copy",
        ]),
        &source,
    );
    let rotated = snapshot(&source);
    assert!(rotated
        .streams
        .iter()
        .find(|stream| stream.kind == media::Type::Video)
        .unwrap()
        .side_data
        .iter()
        .any(|(kind, _)| kind == "DisplayMatrix"));
    let output = directory.path().join("native.mp4");
    let oracle = directory.path().join("oracle.mp4");
    execute_media_plan(
        plan(&source, &output, StreamMode::Copy, StreamMode::Copy),
        None,
    )
    .await
    .unwrap();
    cli(
        &strings(&[
            "-i",
            source.to_str().unwrap(),
            "-c:v",
            "copy",
            "-c:a",
            "copy",
        ]),
        &oracle,
    );
    assert_movie(
        &snapshot(&output),
        &snapshot(&oracle),
        "copy preserves rotation matrix",
    );
    let video = VideoEncodeOptions {
        threads: Some(1),
        gop: Some(120),
        options: [
            ("preset".into(), "medium".into()),
            ("crf".into(), "20".into()),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    let mut request = plan(&source, &output, StreamMode::Video(video), StreamMode::Copy);
    request.outputs[0].duration = Some(0.5);
    execute_media_plan(request, None).await.unwrap();
    cli(
        &strings(&[
            "-i",
            source.to_str().unwrap(),
            "-t",
            "0.5",
            "-c:v",
            "libx264",
            "-threads:v",
            "1",
            "-preset",
            "medium",
            "-crf",
            "20",
            "-g",
            "120",
            "-c:a",
            "copy",
        ]),
        &oracle,
    );
    let actual = snapshot(&output);
    let video = actual
        .streams
        .iter()
        .find(|stream| stream.kind == media::Type::Video)
        .unwrap();
    assert_eq!(video.size, (72, 128));
    assert!(video
        .side_data
        .iter()
        .all(|(kind, _)| kind != "DisplayMatrix"));
    assert_movie(
        &actual,
        &snapshot(&oracle),
        "encode applies rotation to pixels",
    );
    assert_clean(directory.path());
}

mod concat;
mod graphs;
