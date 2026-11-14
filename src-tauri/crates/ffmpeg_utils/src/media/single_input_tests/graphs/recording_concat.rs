//! Shared-graph boundaries for recorded fragments. Changing inputs must be
//! encoded independently before entering a stateful composition graph.
use super::*;

fn fragment(path: &Path, size: &str, color: &str, duration: f64, start: f64, filter: &str) {
    cli(
        &strings(&[
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s={size}:r=20:d={duration}"),
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=440:sample_rate=48000:duration={duration}"),
            "-vf",
            filter,
            "-c:v",
            "libx264",
            "-threads:v",
            "1",
            "-g",
            "20",
            "-bf",
            "2",
            "-c:a",
            "aac",
            "-output_ts_offset",
            &start.to_string(),
            "-f",
            "mpegts",
        ]),
        path,
    );
}

async fn remux(source: &Path, target: &Path) {
    execute_media_plan(
        plan(source, target, StreamMode::Copy, StreamMode::Copy),
        None,
    )
    .await
    .unwrap();
}

async fn join(sources: &[std::path::PathBuf], target: &Path) -> Result<(), String> {
    let mut filters = Vec::new();
    let mut pads = String::new();
    let mut bindings = Vec::new();
    for i in 0..sources.len() {
        filters.push(format!(
            "[{i}:v]scale=160:120:force_original_aspect_ratio=decrease,pad=160:120:(ow-iw)/2:(oh-ih)/2,setsar=1[v{i}];\
             [{i}:a]aresample=async=1:first_pts=0[a{i}]"
        ));
        pads.push_str(&format!("[v{i}][a{i}]"));
        bindings.extend([binding(i, true), binding(i, false)]);
    }
    filters.push(format!("{pads}concat=n={}:v=1:a=1[v][a]", sources.len()));
    execute_media_plan(
        graph_plan(
            sources.iter().map(MediaInput::file).collect(),
            vec![MediaOutput::file(target)],
            &filters.join(";"),
            bindings,
            vec![
                graph_route("v", 0, StreamMode::Video(video_options())),
                graph_route("a", 0, StreamMode::Audio(AudioEncodeOptions::default())),
            ],
            vec![output("v", true), output("a", false)],
        ),
        None,
    )
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

fn assert_duration(target: &Path, expected: f64) {
    let mut measured = Vec::new();
    for stream in snapshot(target).streams {
        assert!(
            stream
                .packets
                .windows(2)
                .all(|pair| pair[0].dts < pair[1].dts),
            "{:?}: output DTS must remain increasing",
            stream.kind
        );
        let duration =
            stream.duration as f64 * stream.time_base.0 as f64 / stream.time_base.1 as f64;
        eprintln!(
            "{:?}: expected {expected:.3}s, actual {duration:.3}s",
            stream.kind
        );
        measured.push((stream.kind, duration));
    }
    for (kind, duration) in measured {
        assert!(
            (duration - expected).abs() < 0.3,
            "{kind:?}: {duration} != {expected}"
        );
    }
}

fn frame_colors(target: &Path) -> (usize, usize) {
    let decoded = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(target)
        .args([
            "-vf",
            "crop=2:2:(iw-2)/2:(ih-2)/2,format=rgb24",
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        decoded.status.success(),
        "{}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    assert!(
        decoded.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    let (frames, remainder) = decoded.stdout.as_chunks::<12>();
    assert!(remainder.is_empty());
    let mut red = 0;
    let mut blue = 0;
    for frame in frames {
        if frame[0] > 200 && frame[2] < 30 {
            red += 1;
        } else if frame[2] > 200 && frame[0] < 30 {
            blue += 1;
        } else {
            panic!("unexpected center pixel: {:?}", &frame[..3]);
        }
    }
    eprintln!("decoded content: {red} red frames, {blue} blue frames");
    (red, blue)
}

#[tokio::test]
async fn seven_recordings_with_different_sizes_and_origins_keep_the_full_duration() {
    assert_seven_recordings(false).await;
}

#[tokio::test]
async fn shared_concat_rejects_repeated_format_changes_without_publishing() {
    assert_seven_recordings(true).await;
}

async fn assert_seven_recordings(change_formats: bool) {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let mut sources = Vec::new();
    let mut expected_duration = 0.0;
    let durations = [4.0, 1.0, 0.4, 1.0, 1.0, 2.0, 2.0];
    for (i, duration) in durations.into_iter().enumerate() {
        let ts = directory.path().join(format!("{i}.ts"));
        let mp4 = directory.path().join(format!("{i}.mp4"));
        let size = if i % 2 == 0 { "160x90" } else { "90x160" };
        let color = if i % 2 == 0 { "red" } else { "blue" };
        let start = 30.0 + i as f64 * 100.0;
        let changes = change_formats && i > 0;
        fragment(
            &ts,
            size,
            color,
            if changes { 0.1 } else { duration },
            start,
            "setsar=1",
        );
        if changes {
            let body = directory.path().join(format!("{i}-body.ts"));
            fragment(
                &body,
                "120x120",
                color,
                duration - 0.1,
                start + 0.1,
                "setsar=2",
            );
            fs::write(
                &ts,
                [fs::read(&ts).unwrap(), fs::read(body).unwrap()].concat(),
            )
            .unwrap();
        }
        remux(&ts, &mp4).await;
        // Each independently encoded AAC fragment adds priming/tail samples.
        // Concat pads to the longest stream of each recording, so compare its
        // duration to those actual inputs, not just the nominal color lengths.
        expected_duration += snapshot(&mp4)
            .streams
            .iter()
            .map(|stream| {
                (stream.start + stream.duration) as f64 * stream.time_base.0 as f64
                    / stream.time_base.1 as f64
            })
            .fold(0.0, f64::max);
        sources.push(mp4);
    }
    let target = directory.path().join("joined.mp4");
    if change_formats {
        fs::write(&target, b"previous complete recording").unwrap();
        let error = join(&sources, &target).await.unwrap_err();
        assert!(error.contains("媒体格式发生变化"), "{error}");
        assert_eq!(fs::read(target).unwrap(), b"previous complete recording");
        assert_clean(directory.path());
        return;
    }
    join(&sources, &target).await.unwrap();
    assert_eq!(frame_colors(&target), (148, 80));
    assert_duration(&target, expected_duration);
}

#[tokio::test]
async fn shared_concat_rejects_an_early_sar_change() {
    assert_fragment_change("90x160", "setsar=2", 0.1).await;
}

#[tokio::test]
async fn shared_concat_rejects_an_early_resolution_change() {
    assert_fragment_change("160x90", "setsar=1", 0.1).await;
}

#[tokio::test]
async fn shared_concat_rejects_a_late_resolution_change_without_publishing() {
    assert_fragment_change("160x90", "setsar=1", 1.9).await;
}

#[tokio::test]
async fn shared_concat_rejects_a_pixel_format_change() {
    assert_fragment_change("90x160", "setsar=1,format=yuv444p", 0.1).await;
}

#[tokio::test]
async fn shared_concat_rejects_a_color_metadata_change() {
    assert_fragment_change(
        "90x160",
        "setsar=1,setparams=colorspace=bt709:range=limited",
        0.1,
    )
    .await;
}

#[tokio::test]
async fn recording_with_stable_fragments_keeps_later_concat_content() {
    assert_fragment_change("90x160", "setsar=1", 0.1).await;
}

async fn assert_fragment_change(body_size: &str, body_filter: &str, change_at: f64) {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.ts");
    let second_head = directory.path().join("second-head.ts");
    let second_body = directory.path().join("second-body.ts");
    fragment(&first, "160x90", "red", 4.0, 30.0, "setsar=1");
    let head_filter = if body_filter.contains("setparams") {
        "setsar=1,setparams=colorspace=bt470bg:range=limited"
    } else {
        "setsar=1"
    };
    fragment(&second_head, "90x160", "blue", change_at, 60.0, head_filter);
    fragment(
        &second_body,
        body_size,
        "blue",
        2.0 - change_at,
        60.0 + change_at,
        body_filter,
    );
    let second = directory.path().join("second.ts");
    fs::write(
        &second,
        [
            fs::read(second_head).unwrap(),
            fs::read(second_body).unwrap(),
        ]
        .concat(),
    )
    .unwrap();
    let sources = [
        directory.path().join("first.mp4"),
        directory.path().join("second.mp4"),
    ];
    remux(&first, &sources[0]).await;
    remux(&second, &sources[1]).await;
    // Prove assembly/remux did not lose content before the shared concat graph.
    assert_duration(&sources[0], 4.0);
    assert_duration(&sources[1], 2.0);
    assert_eq!(frame_colors(&sources[0]), (80, 0));
    assert_eq!(frame_colors(&sources[1]), (0, 40));
    let target = directory.path().join("joined.mp4");
    if body_size != "90x160" || body_filter != "setsar=1" {
        fs::write(&target, b"previous complete recording").unwrap();
        let error = join(&sources, &target).await.unwrap_err();
        assert!(error.contains("媒体格式发生变化"), "{error}");
        assert_eq!(fs::read(target).unwrap(), b"previous complete recording");
        assert_clean(directory.path());
        return;
    }
    join(&sources, &target).await.unwrap();
    let colors = frame_colors(&target);
    assert_duration(&target, 6.0);
    assert_eq!(colors, (80, 40));
}

#[tokio::test]
async fn unsupported_audio_change_fails_without_publishing_partial_concat() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let mut fragments = Vec::new();
    for rate in [48000, 44100] {
        let path = directory.path().join(format!("{rate}.aac"));
        cli(
            &strings(&[
                "-f",
                "lavfi",
                "-i",
                &format!("sine=sample_rate={rate}:duration=0.3"),
                "-c:a",
                "aac",
                "-f",
                "adts",
            ]),
            &path,
        );
        fragments.push(fs::read(path).unwrap());
    }
    let changed = directory.path().join("changed.aac");
    fs::write(&changed, fragments.concat()).unwrap();
    let first = directory.path().join("48000.aac");
    let target = directory.path().join("kept.m4a");
    fs::write(&target, b"previous complete recording").unwrap();
    let error = execute_media_plan(
        graph_plan(
            vec![MediaInput::file(first), MediaInput::file(changed)],
            vec![MediaOutput::file(&target)],
            "[0:a][1:a]concat=n=2:v=0:a=1[a]",
            vec![binding(0, false), binding(1, false)],
            vec![graph_route(
                "a",
                0,
                StreamMode::Audio(AudioEncodeOptions::default()),
            )],
            vec![output("a", false)],
        ),
        None,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("媒体格式发生变化"), "{error}");
    assert_eq!(fs::read(target).unwrap(), b"previous complete recording");
    assert_clean(directory.path());
}

#[tokio::test]
async fn unnormalized_video_change_fails_without_publishing_partial_output() {
    if !has_cli() {
        return;
    }
    for (rotation, size, filter) in [
        (0, "90x160", "setsar=1"),
        (90, "90x160", "setsar=1"),
        (180, "90x160", "setsar=1"),
        (90, "160x90", "setsar=1,format=yuv444p"),
    ] {
        assert_unnormalized_change(rotation, size, filter).await;
    }
}

async fn assert_unnormalized_change(rotation: i32, size: &str, filter: &str) {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.ts");
    let second = directory.path().join("second.ts");
    fragment(&first, "160x90", "red", 1.0, 30.0, "setsar=1");
    fragment(&second, size, "blue", 1.0, 31.0, filter);
    let source = directory.path().join("changed.ts");
    fs::write(
        &source,
        [fs::read(first).unwrap(), fs::read(second).unwrap()].concat(),
    )
    .unwrap();
    let source = if rotation == 0 {
        source
    } else {
        let rotated = directory.path().join("fixed-rotation.mp4");
        cli(
            &strings(&[
                "-display_rotation",
                &rotation.to_string(),
                "-i",
                source.to_str().unwrap(),
                "-c",
                "copy",
            ]),
            &rotated,
        );
        assert!(snapshot(&rotated).streams[0]
            .side_data
            .iter()
            .any(|(kind, _)| kind == "DisplayMatrix"));
        rotated
    };
    let target = directory.path().join("kept.mp4");
    fs::write(&target, b"previous complete recording").unwrap();
    let error = execute_media_plan(
        graph_plan(
            vec![MediaInput::file(source)],
            vec![MediaOutput::file(&target)],
            "[0:v]scale=160:120,setsar=1[v]",
            vec![binding(0, true)],
            vec![graph_route("v", 0, StreamMode::Video(video_options()))],
            vec![output("v", true)],
        ),
        None,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("媒体格式发生变化"), "{error}");
    assert_eq!(fs::read(target).unwrap(), b"previous complete recording");
    assert_clean(directory.path());
}

#[tokio::test]
async fn changed_rotation_fails_without_restarting_the_shared_graph() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.ts");
    let second = directory.path().join("second.ts");
    fragment(&first, "160x90", "red", 1.0, 30.0, "setsar=1");
    fragment(&second, "160x90", "blue", 1.0, 31.0, "setsar=1");
    let rotated = directory.path().join("rotated.ts");
    cli(
        &strings(&[
            "-copyts",
            "-i",
            second.to_str().unwrap(),
            "-c",
            "copy",
            "-bsf:v",
            "h264_metadata=display_orientation=insert:rotate=90",
            "-muxdelay",
            "0",
        ]),
        &rotated,
    );
    let source = directory.path().join("changed.ts");
    fs::write(
        &source,
        [fs::read(first).unwrap(), fs::read(rotated).unwrap()].concat(),
    )
    .unwrap();
    let target = directory.path().join("kept.mp4");
    fs::write(&target, b"previous complete recording").unwrap();
    let error = execute_media_plan(
        graph_plan(
            vec![MediaInput::file(source)],
            vec![MediaOutput::file(&target)],
            "[0:v]scale=160:120,setsar=1[v]",
            vec![binding(0, true)],
            vec![graph_route("v", 0, StreamMode::Video(video_options()))],
            vec![output("v", true)],
        ),
        None,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("旋转信息发生变化"), "{error}");
    assert_eq!(fs::read(target).unwrap(), b"previous complete recording");
    assert_clean(directory.path());
}
