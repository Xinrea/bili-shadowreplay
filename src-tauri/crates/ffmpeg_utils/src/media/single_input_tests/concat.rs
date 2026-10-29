use super::*;

fn concat_plan(list: &Path, output: &Path) -> MediaPlan {
    let mut request = plan(list, output, StreamMode::Copy, StreamMode::Copy);
    request.inputs[0].format = Some("concat".into());
    request.inputs[0].options.insert("safe".into(), "0".into());
    request.routes.push(StreamRoute {
        input: 0,
        output: 0,
        selector: StreamSelector::AutoSubtitle,
        required: false,
        mode: StreamMode::Copy,
    });
    request
}

fn file_entry(path: &Path) -> String {
    format!(
        "file '{}'\n",
        path.to_str()
            .unwrap()
            .replace('\\', "\\\\")
            .replace('\'', "'\\''")
    )
}

fn coded_nals<'a>(packet: &'a CodedPacket, extradata: &[u8]) -> Vec<&'a [u8]> {
    let length_size = usize::from((extradata[4] & 3) + 1);
    let mut bytes = packet.payload.as_slice();
    let mut nals = Vec::new();
    while !bytes.is_empty() {
        assert!(bytes.len() >= length_size);
        let length = bytes[..length_size]
            .iter()
            .fold(0usize, |value, byte| (value << 8) | usize::from(*byte));
        bytes = &bytes[length_size..];
        assert!(length > 0 && length <= bytes.len());
        let nal = &bytes[..length];
        if !matches!(nal[0] & 31, 7 | 8) {
            nals.push(nal);
        }
        bytes = &bytes[length..];
    }
    nals
}

#[tokio::test]
async fn native_concat_copies_both_files_in_order_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/video_frame/fixtures/seek.mp4");
    let list = directory.path().join("list.txt");
    fs::write(&list, file_entry(&source).repeat(2)).unwrap();
    let output = directory.path().join("joined.mp4");
    execute_media_plan(concat_plan(&list, &output), None)
        .await
        .unwrap();
    let expected = snapshot(&source);
    let actual = snapshot(&output);
    assert_eq!(actual.streams.len(), expected.streams.len());
    for (actual, expected) in actual.streams.iter().zip(&expected.streams) {
        assert_eq!(actual.codec, expected.codec);
        assert_eq!(actual.packets.len(), expected.packets.len() * 2);
        for (index, (actual_packet, expected_packet)) in actual
            .packets
            .iter()
            .zip(expected.packets.iter().cycle())
            .enumerate()
        {
            // The concat demuxer inserts SPS/PPS through h264_mp4toannexb.
            // Check every remaining coded NAL instead of mistaking this
            // required container conversion for a payload corruption.
            assert!(
                coded_nals(actual_packet, &actual.extradata)
                    == coded_nals(expected_packet, &expected.extradata),
                "coded packet {index} changed"
            );
        }
    }
    assert_clean(directory.path());
}

#[tokio::test]
async fn native_concat_matches_cli_packets_duration_hints_and_ts_mux_delay() {
    if !has_cli() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("片段 [one]'s.mp4");
    synthetic(&source);
    for hints in [false, true] {
        let list = directory.path().join("list.txt");
        let mut content = file_entry(&source);
        if hints {
            content.push_str("duration 1.500000\n");
        }
        content.push_str(&file_entry(&source));
        fs::write(&list, content).unwrap();
        for extension in ["mp4", "ts"] {
            let output = directory.path().join(format!("native.{extension}"));
            let oracle = directory.path().join(format!("oracle.{extension}"));
            execute_media_plan(concat_plan(&list, &output), None)
                .await
                .unwrap();
            cli(
                &strings(&[
                    "-f",
                    "concat",
                    "-safe",
                    "0",
                    "-i",
                    list.to_str().unwrap(),
                    "-c",
                    "copy",
                ]),
                &oracle,
            );
            assert_movie(
                &snapshot(&output),
                &snapshot(&oracle),
                &format!("concat {extension} duration hints={hints}"),
            );
        }
    }
    assert_clean(directory.path());
}

#[tokio::test]
async fn unsupported_native_input_formats_and_options_preserve_existing_output() {
    let directory = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/video_frame/fixtures/seek.mp4");
    let output = directory.path().join("kept.mp4");
    fs::write(&output, b"previous media").unwrap();
    for (format, options) in [
        (Some("bsr_missing_demuxer"), BTreeMap::new()),
        (
            Some("mov"),
            [("bsr_unused_input_option".into(), "1".into())]
                .into_iter()
                .collect(),
        ),
        (Some("mov\0invalid"), BTreeMap::new()),
        (
            None,
            [("bsr_option".into(), "value\0invalid".into())]
                .into_iter()
                .collect(),
        ),
    ] {
        let mut request = plan(&source, &output, StreamMode::Copy, StreamMode::Copy);
        request.inputs[0].format = format.map(str::to_owned);
        request.inputs[0].options = options;
        assert!(execute_media_plan(request, None).await.is_err());
        assert_eq!(fs::read(&output).unwrap(), b"previous media");
        assert_clean(directory.path());
    }
}

#[tokio::test]
async fn cancelled_concat_retains_its_list_until_the_native_input_closes() {
    use std::{
        io::Write,
        sync::Weak,
        time::{Duration, Instant},
    };
    let retained = Arc::new(Mutex::new(None::<(PathBuf, Weak<tempfile::TempPath>)>));
    let observed = retained.clone();
    crate::audio::opus_tests::assert_cancelled_media(
        move |url, output| async move {
            let mut list = tempfile::Builder::new()
                .prefix(".retained-list-")
                .tempfile_in(output.parent().unwrap())
                .unwrap();
            list.write_all(file_entry(&url).as_bytes()).unwrap();
            list.flush().unwrap();
            let mut input = MediaInput::owned_file(list.into_temp_path());
            input.format = Some("concat".into());
            input.options.insert("safe".into(), "0".into());
            input
                .options
                .insert("protocol_whitelist".into(), "file,http,tcp".into());
            *observed.lock().unwrap() = Some((
                input.path.clone(),
                Arc::downgrade(input._lease.as_ref().unwrap()),
            ));
            let request = MediaPlan {
                inputs: vec![input],
                outputs: vec![MediaOutput::file(output)],
                routes: vec![StreamRoute {
                    input: 0,
                    output: 0,
                    selector: StreamSelector::FirstAudio,
                    required: true,
                    mode: StreamMode::Copy,
                }],
            };
            execute_media_plan(request, None)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        },
        "wav",
        ".media-job-",
    )
    .await;
    let (path, lease) = retained.lock().unwrap().take().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while lease.upgrade().is_some() {
        assert!(
            Instant::now() < deadline,
            "cancelled concat worker retained its list"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!path.exists());
}
