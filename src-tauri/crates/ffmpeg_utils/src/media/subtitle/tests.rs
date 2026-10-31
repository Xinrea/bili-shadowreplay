use super::*;
use ffmpeg_next::packet::Ref;
use std::{fs, path::Path, process::Command, sync::atomic::Ordering};

fn limits() -> Limits {
    Limits {
        input_start: None,
        input_duration: None,
        output_start: 0,
        output_seek: false,
        output_duration: None,
    }
}

fn stage(limits: Limits, cancelled: Arc<AtomicBool>) -> SubtitleStage {
    crate::metadata::initialize().unwrap();
    let mut parameters = codec::Parameters::new();
    parameters.set_medium(ffmpeg::media::Type::Subtitle);
    parameters.set_id(codec::Id::SUBRIP);
    SubtitleStage::new(
        parameters,
        (1, 1000).into(),
        ffmpeg::encoder::find(codec::Id::ASS).unwrap(),
        limits,
        true,
        cancelled,
    )
    .unwrap()
}

fn event(text: &str, pts: Option<i64>, duration: i64) -> Packet {
    let mut packet = Packet::copy(text.as_bytes());
    packet.set_pts(pts);
    packet.set_dts(pts);
    packet.set_duration(duration);
    packet
}

fn packet(stage: &mut SubtitleStage, packet: &Packet) -> Result<Vec<Packet>, String> {
    let mut output = Vec::new();
    stage.packet(packet, &mut |packet, _| {
        output.push(packet);
        Ok(())
    })?;
    Ok(output)
}
fn finish(stage: &mut SubtitleStage) -> Result<Vec<Packet>, String> {
    let mut output = Vec::new();
    stage.finish(&mut |packet, _| {
        output.push(packet);
        Ok(())
    })?;
    Ok(output)
}

#[test]
fn native_subtitle_encoder_retains_ass_header_utf8_styles_and_event_spans_without_a_cli() {
    let mut stage = stage(limits(), Arc::new(AtomicBool::new(false)));
    let parameters = stage.parameters();
    assert_eq!(parameters.id(), codec::Id::ASS);
    let raw = unsafe { &*parameters.as_ptr() };
    let header = unsafe { std::slice::from_raw_parts(raw.extradata, raw.extradata_size as usize) };
    assert!(header
        .windows(b"[V4+ Styles]".len())
        .any(|bytes| bytes == b"[V4+ Styles]"));
    let packets = packet(
        &mut stage,
        &event("Hello <i>字幕</i>\nnext", Some(200), 700),
    )
    .unwrap();
    assert_eq!(packets.len(), 1);
    let packet = &packets[0];
    assert!(
        unsafe { (*(*packet.as_ptr()).buf).size } < 4096,
        "a small queued event must not retain the 1 MiB scratch allocation"
    );
    assert_eq!(
        (packet.pts(), packet.dts(), packet.duration()),
        (Some(200000), Some(200000), 700000)
    );
    assert_eq!(
        String::from_utf8(packet.data().unwrap().to_vec()).unwrap(),
        "0,0,Default,,0,0,0,,Hello {\\i1}字幕{\\i0}\\Nnext"
    );
    assert!(finish(&mut stage).unwrap().is_empty());
    assert!(stage.done());
}

#[test]
fn native_subtitle_output_seek_gates_but_does_not_rebase_or_shorten_events_without_a_cli() {
    let mut constraints = limits();
    constraints.output_seek = true;
    constraints.output_start = 500000;
    constraints.output_duration = Some(1000000);
    constraints.input_start = Some(0);
    constraints.input_duration = Some(100000);
    let mut stage = stage(constraints, Arc::new(AtomicBool::new(false)));
    let mut packets = Vec::new();
    for (pts, duration) in [(200, 700), (700, 1100), (1300, 1100), (1500, 500)] {
        packets.extend(packet(&mut stage, &event("caption", Some(pts), duration)).unwrap());
    }
    assert_eq!(
        packets
            .iter()
            .map(|packet| (packet.pts(), packet.duration()))
            .collect::<Vec<_>>(),
        vec![(Some(700000), 1100000), (Some(1300000), 1100000)]
    );
    assert!(packets[0].data().unwrap().starts_with(b"1,0,Default"));
    assert!(stage.done());
    assert!(finish(&mut stage).unwrap().is_empty());
}

#[test]
fn native_subtitle_missing_pts_cancel_and_encoder_capacity_are_explicit_without_a_cli() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut stage = stage(limits(), cancelled.clone());
    assert!(packet(&mut stage, &event("no timestamp", None, 700))
        .unwrap()
        .is_empty());
    let error = packet(
        &mut stage,
        &event(&"x".repeat(MAX_PACKET as usize + 1), Some(0), 1000),
    )
    .err()
    .unwrap();
    assert!(error.contains("编码字幕失败"), "{error}");
    let error = stage
        .packet(&event("sink failure", Some(1000), 500), &mut |_, _| {
            Err("test subtitle sink failed".into())
        })
        .unwrap_err();
    assert_eq!(error, "test subtitle sink failed");
    cancelled.store(true, Ordering::Release);
    assert!(packet(&mut stage, &event("cancelled", Some(1000), 500)).is_err());
    assert!(finish(&mut stage).is_err());
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    codec: codec::Id,
    base: (i32, i32),
    header: Vec<u8>,
    packets: Vec<SubtitlePacket>,
}

#[derive(Debug, PartialEq, Eq)]
struct SubtitlePacket {
    pts: Option<i64>,
    dts: Option<i64>,
    duration: i64,
    payload: Vec<u8>,
}

fn snapshot(path: &Path) -> Snapshot {
    let mut input = crate::metadata::open_input(path, Arc::new(AtomicBool::new(false))).unwrap();
    let stream = input
        .streams()
        .find(|stream| stream.parameters().medium() == ffmpeg::media::Type::Subtitle)
        .unwrap();
    let index = stream.index();
    let parameters = stream.parameters();
    let raw = unsafe { &*parameters.as_ptr() };
    let mut result = Snapshot {
        codec: parameters.id(),
        base: (
            stream.time_base().numerator(),
            stream.time_base().denominator(),
        ),
        header: if raw.extradata_size > 0 {
            unsafe { std::slice::from_raw_parts(raw.extradata, raw.extradata_size as usize) }
                .to_vec()
        } else {
            Vec::new()
        },
        packets: Vec::new(),
    };
    loop {
        let mut packet = Packet::empty();
        match packet.read(&mut input) {
            Ok(()) if packet.stream() == index => result.packets.push(SubtitlePacket {
                pts: packet.pts(),
                dts: packet.dts(),
                duration: packet.duration(),
                payload: packet.data().unwrap_or_default().to_vec(),
            }),
            Ok(()) => (),
            Err(ffmpeg::Error::Eof) => break,
            Err(error) => panic!("subtitle demux: {error}"),
        }
    }
    result
}

const SRT:&str="1\n00:00:00,200 --> 00:00:00,900\nbefore <i>seek</i>\n\n2\n00:00:00,700 --> 00:00:01,800\nmiddle 中文\n\n3\n00:00:01,300 --> 00:00:02,400\nlast\nline\n\n";

fn plan(input: &Path, output: &Path) -> super::super::MediaPlan {
    use super::super::*;
    MediaPlan {
        inputs: vec![MediaInput::file(input)],
        outputs: vec![MediaOutput::file(output)],
        routes: vec![StreamRoute {
            source: StreamSource::Input {
                input: 0,
                selector: StreamSelector::AutoSubtitle,
            },
            output: 0,

            required: true,
            mode: StreamMode::Subtitle(SubtitleEncodeOptions::default()),
        }],
        ..Default::default()
    }
}

#[tokio::test]
async fn native_subtitle_only_mkv_output_uses_default_ass_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("字幕 源.srt");
    let output = directory.path().join("subtitles.mkv");
    fs::write(&input, SRT).unwrap();
    super::super::execute_media_plan(plan(&input, &output), None)
        .await
        .unwrap();
    let actual = snapshot(&output);
    assert_eq!(actual.codec, codec::Id::ASS);
    assert_eq!(
        actual
            .packets
            .iter()
            .map(|packet| (packet.pts, packet.duration))
            .collect::<Vec<_>>(),
        vec![(Some(200), 700), (Some(700), 1100), (Some(1300), 1100)]
    );
}

#[tokio::test]
async fn native_subtitle_mkv_matches_cli_header_payload_and_input_output_seek_timing() {
    if !Command::new(crate::ffmpeg_path())
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("captions.srt");
    let native = directory.path().join("native.mkv");
    let oracle = directory.path().join("oracle.mkv");
    fs::write(&input, SRT).unwrap();
    for case in ["full", "output seek", "input seek", "input duration"] {
        let mut plan = plan(&input, &native);
        let mut before = Vec::new();
        let mut after = Vec::new();
        match case {
            "output seek" => {
                plan.outputs[0].seek = Some(0.5);
                plan.outputs[0].duration = Some(1.0);
                after.extend(["-ss", "0.5", "-t", "1"]);
            }
            "input seek" => {
                plan.inputs[0].seek = Some(super::super::InputSeek {
                    seconds: 0.5,
                    accurate: true,
                });
                plan.inputs[0].duration = Some(1.0);
                before.extend(["-ss", "0.5", "-t", "1"]);
            }
            "input duration" => {
                plan.inputs[0].duration = Some(1.0);
                before.extend(["-t", "1"]);
            }
            _ => (),
        }
        super::super::execute_media_plan(plan, None)
            .await
            .unwrap_or_else(|error| panic!("{case}: {error}"));
        let result = Command::new(crate::ffmpeg_path())
            .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"])
            .args(before)
            .arg("-i")
            .arg(&input)
            .args(after)
            .args(["-c:s", "ass"])
            .arg(&oracle)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{case}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            snapshot(&native),
            snapshot(&oracle),
            "{case}: complete encoded subtitle header/payload/PTS/DTS/duration"
        );
    }
}

#[tokio::test]
async fn native_ass_styles_header_and_events_survive_default_mkv_encoding_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("styled.ass");
    let output = directory.path().join("styled.mkv");
    let header="[Script Info]\nScriptType: v4.00+\nPlayResX: 640\nPlayResY: 360\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Caption,Arial,23,&H00FFFFFF,&H00000000,&H00000000,&H00000000,-1,0,0,0,100,100,0,0,1,2,1,7,12,13,14,1\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n";
    fs::write(&input,format!("{header}Dialogue: 3,0:00:00.20,0:00:00.90,Caption,作者,12,13,14,,{{\\pos(20,30)}}styled 字幕\n")).unwrap();
    super::super::execute_media_plan(plan(&input, &output), None)
        .await
        .unwrap();
    let actual = snapshot(&output);
    assert!(String::from_utf8_lossy(&actual.header).contains("Style: Caption,Arial,23"));
    assert_eq!(actual.packets.len(), 1);
    assert_eq!(
        (actual.packets[0].pts, actual.packets[0].duration),
        (Some(200), 700)
    );
    let text = String::from_utf8_lossy(&actual.packets[0].payload);
    assert!(
        text.contains(",3,Caption,作者,12,13,14,,{\\pos(20,30)}styled 字幕"),
        "{text}"
    );
    if Command::new(crate::ffmpeg_path())
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        let oracle = directory.path().join("oracle.mkv");
        let result = Command::new(crate::ffmpeg_path())
            .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&input)
            .arg(&oracle)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            actual,
            snapshot(&oracle),
            "ASS original header/styles/events must match the CLI"
        );
    }
}
