use super::*;
use flate2::{write::ZlibEncoder, Compression};
use std::{fs, path::PathBuf, process::Command};

fn budget() -> Budget {
    Budget {
        cancelled: Arc::new(AtomicBool::new(false)),
        deadline: Instant::now() + TIMEOUT,
    }
}

fn crc(data: &[u8]) -> u32 {
    let mut value = !0u32;
    for byte in data {
        value ^= u32::from(*byte);
        for _ in 0..8 {
            value = (value >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(value & 1));
        }
    }
    !value
}

fn compressed(data: &[u8]) -> Vec<u8> {
    let mut compressor = ZlibEncoder::new(Vec::new(), Compression::default());
    compressor.write_all(data).unwrap();
    compressor.finish().unwrap()
}

fn chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut result = (payload.len() as u32).to_be_bytes().to_vec();
    result.extend_from_slice(kind);
    result.extend_from_slice(payload);
    result.extend_from_slice(&crc(&result[4..]).to_be_bytes());
    result
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut header = width.to_be_bytes().to_vec();
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    let stride = width as usize * 4 + 1;
    let mut pixels = vec![0; stride * height as usize];
    for row in pixels.chunks_mut(stride) {
        for (x, pixel) in row[1..].chunks_exact_mut(4).enumerate() {
            pixel.copy_from_slice(&[
                220,
                40 + (x % 40) as u8 * 5,
                25,
                if x < width as usize / 2 { 128 } else { 255 },
            ]);
        }
    }
    let mut result = b"\x89PNG\r\n\x1a\n".to_vec();
    result.extend(chunk(b"IHDR", &header));
    result.extend(chunk(b"IDAT", &compressed(&pixels)));
    result.extend(chunk(b"IEND", &[]));
    result
}

fn with_chunk(mut png: Vec<u8>, kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    png.splice(33..33, chunk(kind, payload));
    png
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/image/fixtures")
        .join(name)
}

fn rgba(path: &Path) -> (u32, u32, Vec<u8>) {
    let decoded = first_frame(fs::read(path).unwrap(), &budget()).unwrap();
    assert_eq!(decoded.format(), ffmpeg::format::Pixel::RGBA);
    let pixels = decoded
        .data(0)
        .chunks(decoded.stride(0))
        .take(decoded.height() as usize)
        .flat_map(|row| row[..decoded.width() as usize * 4].to_vec())
        .collect();
    (decoded.width(), decoded.height(), pixels)
}

fn clean(directory: &Path) {
    assert!(fs::read_dir(directory).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".bsr-image-")));
}

#[tokio::test]
async fn native_png_preserves_alpha_and_bounds_dimensions_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("输入 图像.png");
    let output = directory.path().join("normalized.png");
    for (width, height, target_width, target_height) in [
        (16, 12, 16, 12),
        (1024, 256, 512, 128),
        (256, 1024, 128, 512),
        (2048, 2048, 512, 512),
    ] {
        fs::write(&input, png(width, height)).unwrap();
        normalize_image_to_png(&input, &output).await.unwrap();
        let (actual_width, actual_height, pixels) = rgba(&output);
        assert_eq!((actual_width, actual_height), (target_width, target_height));
        if width == target_width && height == target_height {
            assert_eq!(&pixels[..4], &[220, 40, 25, 128]);
        } else {
            // Bicubic downscaling rounds color/alpha near the edges. Assert
            // preservation of a translucent region; strict CLI tests below
            // compare the complete resampling result, with no tolerance.
            assert!(pixels[3] > 0 && pixels[3] < 255);
        }
        assert_eq!(pixels[pixels.len() - 1], 255);
        clean(directory.path());
    }
}

#[tokio::test]
async fn native_png_normalizes_first_animation_frame_and_supported_images_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("normalized.png");
    for name in ["animated.png", "alpha.webp", "animated.gif", "still.jpg"] {
        normalize_image_to_png(&fixture(name), &output)
            .await
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let (width, height, pixels) = rgba(&output);
        assert_eq!((width, height), (16, 12));
        assert!(
            pixels[0] > pixels[2],
            "{name}: first frame is red; the second animation frame is blue"
        );
        if name == "animated.png" || name == "alpha.webp" {
            assert_eq!(&pixels[..4], &[220, 40, 25, 128]);
        }
    }
    // The former whitelist excludes newer dedicated animated-WebP demuxers.
    // FFmpeg 6/9's webp_pipe decoder cannot decode this animation; failure must
    // retain the previous PNG so the application can fall back to text.
    let retained = fs::read(&output).unwrap();
    assert!(normalize_image_to_png(&fixture("animated.webp"), &output)
        .await
        .is_err());
    assert_eq!(fs::read(&output).unwrap(), retained);
    clean(directory.path());
}

#[tokio::test]
async fn native_png_rejects_unsafe_payloads_and_keeps_previous_output_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.png");
    let output = directory.path().join("kept.png");
    fs::write(&output, b"previous PNG").unwrap();
    for data in [
        b"#EXTM3U\nfile:///tmp/private\n".to_vec(),
        b"\x89PNG\r\n\x1a\ncorrupt".to_vec(),
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/video_frame/fixtures/seek.mp4"))
            .unwrap(),
        vec![0; MAX_BYTES + 1],
        png(2049, 2048),
    ] {
        fs::write(&input, data).unwrap();
        assert!(normalize_image_to_png(&input, &output).await.is_err());
        assert_eq!(fs::read(&output).unwrap(), b"previous PNG");
        clean(directory.path());
    }
    assert!(normalize_image_to_png(directory.path(), &output)
        .await
        .is_err());
    assert!(
        normalize_image_to_png(Path::new("https://localhost/image.png"), &output)
            .await
            .is_err()
    );
    fs::write(&input, png(16, 12)).unwrap();
    let original = fs::read(&input).unwrap();
    assert!(normalize_image_to_png(&input, &input).await.is_err());
    assert_eq!(fs::read(&input).unwrap(), original);
    let blocked = directory.path().join("directory.png");
    fs::create_dir(&blocked).unwrap();
    assert!(normalize_image_to_png(&input, &blocked).await.is_err());
    clean(directory.path());
}

#[tokio::test]
async fn native_image_headers_enforce_pixel_limit_before_codec_allocation_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("oversized.image");
    let output = directory.path().join("kept.png");
    fs::write(&output, b"previous PNG").unwrap();
    let mut jpeg = fs::read(fixture("still.jpg")).unwrap();
    let sof = jpeg
        .windows(2)
        .position(|bytes| matches!(bytes, [0xff, 0xc0..=0xc3]))
        .unwrap();
    jpeg[sof + 5..sof + 9].fill(0xff);
    let mut webp = b"RIFF".to_vec();
    webp.extend_from_slice(&22u32.to_le_bytes());
    webp.extend_from_slice(b"WEBPVP8X");
    webp.extend_from_slice(&10u32.to_le_bytes());
    webp.extend_from_slice(&[0, 0, 0, 0, 0xfe, 0xff, 0, 0xfe, 0xff, 0]);
    for data in [b"GIF89a\xff\xff\xff\xff".to_vec(), jpeg, webp] {
        fs::write(&input, data).unwrap();
        let error = normalize_image_to_png(&input, &output).await.unwrap_err();
        assert!(error.contains("pixel limit"), "{error}");
        assert_eq!(fs::read(&output).unwrap(), b"previous PNG");
    }
}

#[tokio::test]
async fn native_png_checks_compressed_metadata_before_decode_and_preserves_normal_icc_without_a_cli(
) {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("profile.png");
    let output = directory.path().join("normalized.png");
    let mut profile = vec![0; 256];
    profile[..4].copy_from_slice(&256u32.to_be_bytes());
    profile[36..40].copy_from_slice(b"acsp");
    let mut payload = b"Test profile\0\0".to_vec();
    payload.extend(compressed(&profile));
    let source = with_chunk(png(16, 12), b"iCCP", &payload);
    fs::write(&input, &source).unwrap();
    normalize_image_to_png(&input, &output).await.unwrap();
    assert_eq!(
        fs::read(&input).unwrap(),
        source,
        "preflight must not rewrite original chunks"
    );
    let frame = first_frame(fs::read(&output).unwrap(), &budget()).unwrap();
    assert_eq!(
        frame
            .side_data(frame::side_data::Type::IccProfile)
            .unwrap()
            .data(),
        profile
    );
    let retained = fs::read(&output).unwrap();
    for kind in [b"iCCP", b"zTXt", b"iTXt"] {
        let mut payload = if kind == b"iTXt" {
            b"metadata\0\x01\0\0\0".to_vec()
        } else {
            b"metadata\0\0".to_vec()
        };
        payload.extend(compressed(&vec![0; MAX_BYTES + 1]));
        let source = with_chunk(png(16, 12), kind, &payload);
        assert!(source.len() < MAX_BYTES);
        fs::write(&input, source).unwrap();
        let error = normalize_image_to_png(&input, &output).await.unwrap_err();
        assert!(error.contains("metadata exceeds"), "{error}");
        assert_eq!(fs::read(&output).unwrap(), retained);
    }
    let malformed = with_chunk(png(16, 12), b"iCCP", b"metadata\0\0invalid zlib");
    fs::write(&input, malformed).unwrap();
    assert!(normalize_image_to_png(&input, &output).await.is_err());
    clean(directory.path());
}

#[tokio::test]
async fn native_png_deadline_and_cancellation_prevent_publication_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.png");
    let output = directory.path().join("kept.png");
    fs::write(&input, png(16, 12)).unwrap();
    fs::write(&output, b"previous PNG").unwrap();
    assert!(normalize_with_timeout(&input, &output, Duration::ZERO)
        .await
        .unwrap_err()
        .contains("timed out"));
    assert_eq!(fs::read(&output).unwrap(), b"previous PNG");
    let token = budget();
    let mut reader = ImageReader {
        data: Cursor::new(png(16, 12)),
        budget: token.clone(),
    };
    let mut bytes = [0; 8];
    reader.read_exact(&mut bytes).unwrap();
    token.cancelled.store(true, Ordering::Release);
    assert!(reader.read(&mut bytes).is_err());
    assert!(reader.seek(SeekFrom::Start(0)).is_err());
    assert!(first_frame(png(16, 12), &token).is_err());
    clean(directory.path());
}

#[test]
fn dropped_image_future_cannot_publish_queued_native_work_without_a_cli() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("source.png");
    let output = directory.path().join("kept.png");
    fs::write(&input, png(16, 12)).unwrap();
    fs::write(&output, b"previous PNG").unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let (release, held) = std::sync::mpsc::channel();
    let (started, ready) = std::sync::mpsc::channel();
    let blocker = runtime.spawn_blocking(move || {
        started.send(()).unwrap();
        held.recv().unwrap();
    });
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    let target = output.clone();
    runtime.block_on(async move {
        let task = tokio::spawn(async move { normalize_image_to_png(&input, &target).await });
        // The only blocking worker is held, so normalization is queued while
        // its async owner is alive. Aborting must cancel that queued work too.
        tokio::time::sleep(Duration::from_millis(10)).await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        release.send(()).unwrap();
        blocker.await.unwrap();
    });
    drop(runtime); // waits for queued native work to observe cancellation
    assert_eq!(fs::read(&output).unwrap(), b"previous PNG");
    clean(directory.path());
}

fn cli_available() -> bool {
    Command::new(crate::ffmpeg_path())
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success())
}

async fn assert_cli(source: &Path, directory: &Path) {
    let expected = directory.join("oracle.png");
    let actual = directory.join("native.png");
    let _ = fs::remove_file(&expected);
    let _ = fs::remove_file(&actual);
    let oracle = Command::new(crate::ffmpeg_path())
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-max_alloc",
            "67108864",
            "-protocol_whitelist",
            "file",
            "-format_whitelist",
            FORMATS,
            "-max_pixels",
            "4194304",
            "-threads",
            "1",
            "-i",
        ])
        .arg(source)
        .args([
            "-vf",
            FILTER,
            "-frames:v",
            "1",
            "-threads",
            "1",
            "-update",
            "1",
            "-y",
        ])
        .arg(&expected)
        .output()
        .unwrap();
    let native = normalize_image_to_png(source, &actual).await;
    let expected = fs::read(&expected).unwrap_or_default();
    if !oracle.status.success() || expected.is_empty() {
        assert!(
            native.is_err(),
            "{}: CLI failed but native succeeded",
            source.display()
        );
    } else {
        native.unwrap_or_else(|error| panic!("{}: {error}", source.display()));
        let actual = fs::read(&actual).unwrap();
        assert!(
            actual == expected,
            "{}: PNG bytes differ (native {}, CLI {}, first diff {:?})",
            source.display(),
            actual.len(),
            expected.len(),
            actual.iter().zip(&expected).position(|(a, b)| a != b)
        );
    }
}

#[tokio::test]
async fn native_png_matches_cli_alpha_animation_webp_jpeg_scaling_and_icc() {
    if !cli_available() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    for name in [
        "animated.png",
        "animated.gif",
        "alpha.webp",
        "animated.webp",
        "still.jpg",
    ] {
        assert_cli(&fixture(name), directory.path()).await;
    }
    let source = directory.path().join("source.png");
    for (width, height) in [(16, 12), (1024, 256), (256, 1024), (513, 511)] {
        fs::write(&source, png(width, height)).unwrap();
        assert_cli(&source, directory.path()).await;
    }
    let mut profile = vec![0; 256];
    profile[..4].copy_from_slice(&256u32.to_be_bytes());
    profile[36..40].copy_from_slice(b"acsp");
    let mut payload = b"Test profile\0\0".to_vec();
    payload.extend(compressed(&profile));
    fs::write(&source, with_chunk(png(16, 12), b"iCCP", &payload)).unwrap();
    assert_cli(&source, directory.path()).await;
}
