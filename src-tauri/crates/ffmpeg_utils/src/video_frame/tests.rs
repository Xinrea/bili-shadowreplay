use super::*;
use std::{fs, process::Command};

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/video_frame/fixtures/seek.mp4")
}

fn ppm(path: &Path, width: usize, height: usize) {
    let mut data = format!("P6\n{width} {height}\n255\n").into_bytes();
    for y in 0..height {
        for x in 0..width {
            data.extend_from_slice(&[
                (x * 13 + y * 3) as u8,
                (x * 7 + y * 17) as u8,
                (x * 29 + y * 11) as u8,
            ]);
        }
    }
    fs::write(path, data).unwrap();
}

fn jpeg(data: &[u8]) {
    assert!(data.starts_with(&[0xff, 0xd8]));
    assert!(data.ends_with(&[0xff, 0xd9]));
}

fn clean(directory: &Path) {
    assert!(fs::read_dir(directory).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".bsr-frame-")));
}

#[tokio::test]
async fn native_jpeg_quality_and_seek_work_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("色彩 样本.ppm");
    ppm(&source, 64, 48);
    let default = extract_jpeg(&source, 0.0, None).await.unwrap();
    let q2 = extract_jpeg(&source, 0.0, Some(2)).await.unwrap();
    jpeg(&default);
    jpeg(&q2);
    assert_ne!(
        default, q2,
        "default rate control must not silently become q2"
    );
    let beginning = extract_jpeg(&fixture(), 0.0, Some(2)).await.unwrap();
    let middle = extract_jpeg(&fixture(), 0.45, Some(2)).await.unwrap();
    let final_frame = extract_jpeg(&fixture(), 1.0, Some(2)).await.unwrap();
    jpeg(&beginning);
    jpeg(&middle);
    jpeg(&final_frame);
    assert_ne!(beginning, middle);
    assert_ne!(middle, final_frame);
    assert!(extract_jpeg(&fixture(), 1.2, None).await.is_err());
}

#[tokio::test]
async fn native_jpeg_validation_and_atomic_output_work_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("frame.ppm");
    let target = directory.path().join("frame.jpg");
    ppm(&source, 32, 24);
    fs::write(&target, b"previous thumbnail").unwrap();
    for timestamp in [f64::NAN, f64::INFINITY, -0.01, f64::MAX, 100.0] {
        assert!(write_jpeg(&source, timestamp, None, &target).await.is_err());
        assert_eq!(fs::read(&target).unwrap(), b"previous thumbnail");
        clean(directory.path());
    }
    for quality in [0, 32, u32::MAX] {
        assert!(write_jpeg(&source, 0.0, Some(quality), &target)
            .await
            .is_err());
        assert_eq!(fs::read(&target).unwrap(), b"previous thumbnail");
    }
    let original = fs::read(&source).unwrap();
    assert!(write_jpeg(&source, 0.0, None, &source).await.is_err());
    assert_eq!(fs::read(&source).unwrap(), original);
    #[cfg(unix)]
    {
        let alias = directory.path().join("alias.ppm");
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        assert!(write_jpeg(&source, 0.0, None, &alias).await.is_err());
        assert_eq!(fs::read(&source).unwrap(), original);
    }
    let invalid = directory.path().join("invalid.mp4");
    fs::write(&invalid, b"not media").unwrap();
    assert!(write_jpeg(&invalid, 0.0, None, &target).await.is_err());
    assert_eq!(fs::read(&target).unwrap(), b"previous thumbnail");
    let audio = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/audio/test.wav");
    assert!(write_jpeg(&audio, 0.0, None, &target).await.is_err());
    assert_eq!(fs::read(&target).unwrap(), b"previous thumbnail");
    write_jpeg(&source, 0.0, Some(2), &target).await.unwrap();
    assert_eq!(
        fs::read(&target).unwrap(),
        extract_jpeg(&source, 0.0, Some(2)).await.unwrap()
    );
    let blocked = directory.path().join("directory.jpg");
    fs::create_dir(&blocked).unwrap();
    assert!(write_jpeg(&source, 0.0, None, &blocked).await.is_err());
    clean(directory.path());
}

#[tokio::test]
async fn native_jpeg_pre_cancel_and_concurrent_extracts_work_without_a_cli() {
    assert!(extract_sync(&fixture(), 0, None, Arc::new(AtomicBool::new(true))).is_err());
    let source = fixture();
    let (first, second) = tokio::join!(
        extract_jpeg(&source, 0.0, Some(2)),
        extract_jpeg(&source, 0.7, Some(2))
    );
    let (first, second) = (first.unwrap(), second.unwrap());
    jpeg(&first);
    jpeg(&second);
    assert_ne!(first, second);
}

#[tokio::test]
async fn cancelled_jpeg_read_interrupts_worker_and_keeps_previous_output_without_a_cli() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::oneshot,
        time::{timeout, Duration},
    };
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("kept.jpg");
    fs::write(&target, b"previous thumbnail").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let source = std::path::PathBuf::from(format!(
        "http://{}/stall.mp4",
        listener.local_addr().unwrap()
    ));
    let (ready, received) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut connection, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            connection.read_exact(&mut byte).await.unwrap();
            request.extend_from_slice(&byte);
        }
        connection.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\nContent-Type: video/mp4\r\nConnection: close\r\n\r\n").await.unwrap();
        ready.send(()).unwrap();
        let mut byte = [0];
        assert_eq!(
            timeout(Duration::from_secs(5), connection.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    });
    let task_target = target.clone();
    let task = tokio::spawn(async move { write_jpeg(&source, 0.0, None, &task_target).await });
    timeout(Duration::from_secs(5), received)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"previous thumbnail");
    clean(directory.path());
}

fn has_cli() -> bool {
    Command::new(crate::ffmpeg_path())
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn cli(args: &[&str], output: &Path) {
    let result = Command::new(crate::ffmpeg_path())
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .arg(output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "CLI {:?}: {}",
        args,
        String::from_utf8_lossy(&result.stderr)
    );
}

async fn assert_oracle(source: &Path, timestamp: f64, quality: Option<u32>, directory: &Path) {
    let expected = directory.join("oracle.jpg");
    let _ = fs::remove_file(&expected);
    let timestamp_arg = timestamp.to_string();
    // FFmpeg 6's image2 seek(0) discards its sole image. Native deliberately
    // uses the fixed FFmpeg 9 behavior for stills; compare their encoding using
    // the CLI's equivalent no-seek form. Video always retains the old -ss.
    let still_at_zero = timestamp == 0.0 && source.extension().is_some_and(|ext| ext == "ppm");
    let mut args = Vec::new();
    if !still_at_zero {
        args.extend(["-ss", timestamp_arg.as_str()]);
    }
    args.extend(["-i", source.to_str().unwrap(), "-vframes", "1"]);
    let quality_arg = quality.map(|value| value.to_string());
    if let Some(value) = quality_arg.as_deref() {
        args.extend(["-q:v", value]);
    }
    let result = Command::new(crate::ffmpeg_path())
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"])
        .args(&args)
        .arg(&expected)
        .output()
        .unwrap();
    let actual = extract_jpeg(source, timestamp, quality).await;
    let expected = fs::read(&expected).unwrap_or_default();
    if !result.status.success() || expected.is_empty() {
        assert!(
            actual.is_err(),
            "{} at {timestamp}: CLI had no frame, native succeeded",
            source.display()
        );
    } else {
        let actual = actual.unwrap();
        assert!(actual == expected, "{} at {timestamp}, quality {quality:?}: native {} bytes, CLI {} bytes, first difference {:?}", source.display(), actual.len(), expected.len(), actual.iter().zip(&expected).position(|(a,b)| a != b));
    }
}

#[tokio::test]
async fn native_jpeg_matches_cli_quality_color_sar_and_bframe_seek() {
    if !has_cli() {
        eprintln!("skip JPEG CLI oracle: ffmpeg unavailable");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("rgb.ppm");
    ppm(&source, 64, 48);
    for quality in [None, Some(2)] {
        assert_oracle(&source, 0.0, quality, directory.path()).await;
        for timestamp in [0.0, 0.04, 0.1, 0.1000009, 0.45, 0.9, 1.0] {
            assert_oracle(&fixture(), timestamp, quality, directory.path()).await;
        }
    }
}

#[tokio::test]
async fn native_jpeg_matches_cli_rotations_offsets_and_automatic_video_mapping() {
    if !has_cli() {
        eprintln!("skip JPEG CLI oracle: ffmpeg unavailable");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    for angle in [0, 90, 180, 270, 45] {
        let source = directory.path().join(format!("rotated-{angle}.mp4"));
        cli(
            &[
                "-i",
                fixture().to_str().unwrap(),
                "-c",
                "copy",
                "-metadata:s:v:0",
                &format!("rotate={angle}"),
            ],
            &source,
        );
        for quality in [None, Some(2)] {
            assert_oracle(&source, 0.45, quality, directory.path()).await;
        }
    }
    for offset in [2.0, -0.3] {
        let source = directory.path().join("offset.mp4");
        cli(
            &[
                "-i",
                fixture().to_str().unwrap(),
                "-c",
                "copy",
                "-output_ts_offset",
                &offset.to_string(),
                "-avoid_negative_ts",
                "disabled",
            ],
            &source,
        );
        for timestamp in [0.0, 0.45, 0.9] {
            assert_oracle(&source, timestamp, Some(2), directory.path()).await;
        }
    }
    let source = directory.path().join("multitrack.mp4");
    cli(
        &[
            "-i",
            fixture().to_str().unwrap(),
            "-i",
            fixture().to_str().unwrap(),
            "-map",
            "0:v",
            "-map",
            "1:v",
            "-c",
            "copy",
            "-disposition:v:0",
            "0",
            "-disposition:v:1",
            "default",
            "-metadata:s:v:1",
            "rotate=90",
        ],
        &source,
    );
    assert_oracle(&source, 0.45, Some(2), directory.path()).await;

    // A cover marked default must not outrank actual video, regardless of its
    // area or the demuxer's NEW_PACKETS event flag.
    let cover_source = directory.path().join("cover.ppm");
    let cover = directory.path().join("cover.jpg");
    ppm(&cover_source, 320, 240);
    write_jpeg(&cover_source, 0.0, Some(2), &cover)
        .await
        .unwrap();
    let source = directory.path().join("with-cover.mp4");
    cli(
        &[
            "-i",
            fixture().to_str().unwrap(),
            "-i",
            cover.to_str().unwrap(),
            "-map",
            "0:v",
            "-map",
            "1:v",
            "-c",
            "copy",
            "-disposition:v:0",
            "0",
            "-disposition:v:1",
            "attached_pic+default",
        ],
        &source,
    );
    assert_oracle(&source, 0.45, Some(2), directory.path()).await;
}
