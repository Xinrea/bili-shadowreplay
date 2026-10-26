//! Burn text and image danmaku using one shared layout. Images are downloaded
//! once per job and decoded before layout so failures can fall back to text.
//! Each is kept under a name of its URL, so a directory kept between jobs, as a
//! project's segment cache is, has those of the last job already.

use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU32,
    path::{Path, PathBuf},
    process::Stdio,
    sync::OnceLock,
    time::Duration,
};

use danmu_stream::EmojiContent;
use ffmpeg_utils::ffmpeg_command;
use futures::{stream, StreamExt};
use recorder::danmu::DanmuEntry;
use sha2::{Digest, Sha256};
use url::Url;

use crate::{
    constants,
    danmu2ass::{
        danmu_render_plan, image_height, Danmu2AssOptions, DanmuImage, DanmuRenderPlan, ImageDanmu,
        PLAY_RES_Y,
    },
    progress::progress_reporter::ProgressReporterTrait,
};

use super::{
    hwaccel,
    native_overlay::{add_overlay_input, image_events, run_overlay_command, OverlaySpec},
    runner::{ProgressMode, ProgressTotal, TempArtifact},
};

const MIN_DANMU_FPS: f64 = 60.0;

/// FFmpeg 7 still accepts `-filter_complex_script`. FFmpeg 9 removed it; the
/// replacement `-/filter_complex <file>` reads the same graph from a file, which
/// keeps long danmaku tracks off the OS command-line limit.
fn select_filter_complex_file_option(help: &str) -> &'static str {
    if help.contains("-filter_complex_script") {
        "-filter_complex_script"
    } else {
        "-/filter_complex"
    }
}

pub(super) async fn filter_complex_file_option() -> &'static str {
    static OPTION: OnceLock<&'static str> = OnceLock::new();
    if let Some(option) = OPTION.get() {
        return option;
    }

    let help = match ffmpeg_command()
        .args(["-hide_banner", "-h", "full"])
        .output()
        .await
    {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            text
        }
        Err(error) => {
            log::warn!("Failed to query ffmpeg options; using -/filter_complex: {error}");
            String::new()
        }
    };
    let option = select_filter_complex_file_option(&help);
    let _ = OPTION.set(option);
    OPTION.get().copied().unwrap_or(option)
}
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const IMAGE_TIMEOUT: Duration = Duration::from_secs(10);
// Keep in sync with src/lib/danmu-content.ts. Never let recorded URLs request
// arbitrary services or let an allowed CDN redirect to a local/network URL.
const EMOJI_CDN_DOMAINS: &[&str] = &["hdslb.com", "biliimg.com"];

pub(super) fn danmu_output_fps(source_fps: f64) -> f64 {
    if source_fps.is_finite() && source_fps > 0.0 {
        source_fps.max(MIN_DANMU_FPS)
    } else {
        MIN_DANMU_FPS
    }
}

fn allowed_image_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.host_str().is_some_and(|host| {
            EMOJI_CDN_DOMAINS
                .iter()
                .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
        })
}

fn image_url(emoji: &EmojiContent) -> Option<Url> {
    let raw = emoji.url.trim();
    let mut url = Url::parse(&if raw.starts_with("//") {
        format!("https:{raw}")
    } else {
        raw.to_owned()
    })
    .ok()?;
    if !allowed_image_url(&url) {
        return None;
    }
    url.set_scheme("https").ok()?;
    url.set_fragment(None);
    Some(url)
}

fn image_size(emoji: &EmojiContent, font_size: f64) -> (f64, f64) {
    let ratio = match (emoji.width, emoji.height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => width as f64 / height as f64,
        _ => 1.0,
    };
    let height = image_height(font_size);
    (height * ratio.clamp(0.25, 3.0), height)
}

fn image_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(5))
        .timeout(IMAGE_TIMEOUT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 3
                || attempt.url().scheme() != "https"
                || !allowed_image_url(attempt.url())
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
}

// Restrict FFmpeg to image payloads, not playlists or other formats capable of
// opening additional files/URLs. Do not trust the response MIME type or suffix.
fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("jpg")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else {
        None
    }
}

/// The file the image from `url` is decoded into in a job's directory.
pub(super) fn image_file_name(url: &Url) -> String {
    let digest: String = Sha256::digest(url.as_str().as_bytes())
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("image-{digest}.png")
}

/// Download and decode the image from `url` into `directory`, unless it is
/// there already.
async fn download_image(
    client: &reqwest::Client,
    url: &Url,
    directory: &Path,
    index: usize,
) -> Result<PathBuf, String> {
    let output = directory.join(image_file_name(url));
    if tokio::fs::try_exists(&output).await.unwrap_or(false) {
        return Ok(output);
    }
    let mut response = client
        .get(url.clone())
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("image HTTP status {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_IMAGE_BYTES as u64)
    {
        return Err("image exceeds the download limit".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if bytes.len() + chunk.len() > MAX_IMAGE_BYTES {
            return Err("image exceeds the download limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let extension = image_extension(&bytes).ok_or("unsupported image format")?;
    let source = directory.join(format!("download-{index}.{extension}"));
    tokio::fs::write(&source, bytes)
        .await
        .map_err(|error| error.to_string())?;
    // Under its own name once whole, so that a stopped job leaves none half.
    let decoded = directory.join(format!("decoded-{index}.png"));
    let result = decode_image(&source, &decoded).await;
    let _ = tokio::fs::remove_file(&source).await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(&decoded).await;
        return Err(error);
    }
    tokio::fs::rename(&decoded, &output)
        .await
        .map_err(|error| error.to_string())?;
    Ok(output)
}

/// Normalize the first decodable frame to a bounded PNG with alpha. Some
/// FFmpeg versions cannot decode animated WebP; those images fall back to text.
/// The PNG can be held by FFmpeg without a looping decoder.
async fn decode_image(source: &Path, output: &Path) -> Result<(), String> {
    let mut command = ffmpeg_command();
    command
        .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
        .args(["-max_alloc", "67108864", "-protocol_whitelist", "file"])
        .args(["-format_whitelist", "image2,png_pipe,jpeg_pipe,webp_pipe,gif,apng"])
        .args(["-max_pixels", "4194304", "-threads", "1", "-i"])
        .arg(source)
        .args([
            "-vf", "scale=w='min(iw,512)':h='min(ih,512)':force_original_aspect_ratio=decrease,format=rgba",
            "-frames:v", "1", "-threads", "1", "-update", "1", "-y",
        ])
        .arg(output)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let result = tokio::time::timeout(IMAGE_TIMEOUT, command.output())
        .await
        .map_err(|_| "image decode timed out".to_string())?
        .map_err(|error| error.to_string())?;
    if !result.status.success() {
        return Err(format!(
            "image decode failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    if tokio::fs::metadata(output)
        .await
        .map_err(|error| error.to_string())?
        .len()
        == 0
    {
        return Err("image decoder produced no frame".into());
    }
    Ok(())
}

fn image_urls(danmus: &[DanmuEntry]) -> Vec<Url> {
    let mut seen = HashSet::new();
    danmus
        .iter()
        .filter(|entry| entry.is_danmu() && entry.ts >= 0)
        .filter_map(|entry| entry.emoji_content.as_ref().and_then(image_url))
        .filter(|url| seen.insert(url.clone()))
        .collect()
}

async fn prepare_images(danmus: &[DanmuEntry], directory: &Path) -> HashMap<Url, PathBuf> {
    let urls = image_urls(danmus);
    if urls.is_empty() {
        return HashMap::new();
    }
    let client = match image_client() {
        Ok(client) => client,
        Err(error) => {
            log::warn!("Cannot prepare image danmaku; using text: {error}");
            return HashMap::new();
        }
    };
    stream::iter(urls.into_iter().enumerate())
        .map(|(index, url)| {
            let client = &client;
            async move {
                match download_image(client, &url, directory, index).await {
                    Ok(path) => Some((url, path)),
                    Err(error) => {
                        log::warn!("Cannot load danmaku image; using text: {error}");
                        None
                    }
                }
            }
        })
        .buffer_unordered(4)
        .filter_map(|result| async { result })
        .collect()
        .await
}

fn overlay_graph(fps: f64, vaapi: bool) -> String {
    // Change FPS before compositing, not only at the encoder, so motion is
    // actually evaluated at 60+ FPS rather than duplicating already-drawn frames.
    let mut filter = format!(
        "[0:v]{},fps={fps}[base];\n\
         [base][1:v]overlay=shortest=1:format=auto:alpha=straight[composited];\n",
        hwaccel::H264_SCALE_PAD_FILTER,
    );
    let format = if vaapi {
        hwaccel::vaapi_filter_suffix()
    } else {
        "format=yuv420p"
    };
    filter.push_str(&format!("[composited]{format}[outv]\n"));
    filter
}

fn validate_options(options: &Danmu2AssOptions) -> Result<(), String> {
    if !options.font_size.is_finite()
        || options.font_size <= 0.0
        || options.font_size > PLAY_RES_Y / 1.5
        || !options.opacity.is_finite()
        || !(0.0..=1.0).contains(&options.opacity)
    {
        return Err("Invalid danmaku font size or opacity".into());
    }
    Ok(())
}

/// Download image danmaku and lay out the track with them.
async fn danmu_plan(
    danmus: &[DanmuEntry],
    options: &Danmu2AssOptions,
    directory: &Path,
) -> Result<DanmuRenderPlan, String> {
    validate_options(options)?;
    let assets = prepare_images(danmus, directory).await;
    Ok(danmu_render_plan(danmus, options, |entry| {
        let emoji = entry.emoji_content.as_ref()?;
        let path = assets.get(&image_url(emoji)?)?.clone();
        let (width, height) = image_size(emoji, options.font_size);
        Some(DanmuImage {
            path,
            width,
            height,
        })
    }))
}

/// Lay out the danmaku track into `danmu.ass` in `directory` and return the
/// image danmaku to overlay. Filters must run with `directory` as the working
/// directory.
pub(super) async fn prepare_danmu_layers<R: ProgressReporterTrait>(
    reporter: Option<&R>,
    danmus: &[DanmuEntry],
    options: &Danmu2AssOptions,
    directory: &Path,
) -> Result<Vec<ImageDanmu>, String> {
    if let Some(reporter) = reporter {
        reporter.update("准备弹幕和表情图片中").await;
    }
    let plan = danmu_plan(danmus, options, directory).await?;
    tokio::fs::write(directory.join("danmu.ass"), plan.ass)
        .await
        .map_err(|error| error.to_string())?;
    Ok(plan.images)
}

/// Burn `danmus` into `file`. `bitrate` (kbps) overrides the encoder's default
/// quality.
pub async fn encode_video_danmu<R: ProgressReporterTrait>(
    reporter: Option<&R>,
    file: &Path,
    danmus: &[DanmuEntry],
    options: &Danmu2AssOptions,
    bitrate: Option<NonZeroU32>,
) -> Result<PathBuf, String> {
    // Use fixed relative names inside a private job directory. No user/CDN path
    // is interpolated into filter syntax (including Windows drive letters).
    let directory = tempfile::Builder::new()
        .prefix("bili-danmu-")
        .tempdir()
        .map_err(|error| error.to_string())?;
    if let Some(reporter) = reporter {
        reporter.update("准备弹幕和表情图片中").await;
    }
    let plan = danmu_plan(danmus, options, directory.path()).await?;
    encode_danmu_plan(reporter, file, plan, options, bitrate, directory.path()).await
}

/// FFmpeg rebases all streams against the container start, not the first
/// video frame. A delayed video can therefore end later than its track length.
/// Cover that timeline (and let the primary video end the overlay naturally).
fn overlay_timeline_duration(probe: &serde_json::Value, track_duration: f64) -> f64 {
    let number = |value: &serde_json::Value| {
        value
            .as_str()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
    };
    let format = &probe["format"];
    let container_start = number(&format["start_time"]).unwrap_or(0.0);
    let video_start = probe["streams"]
        .as_array()
        .and_then(|streams| {
            streams
                .iter()
                .find(|stream| stream["codec_type"] == "video")
        })
        .and_then(|video| number(&video["start_time"]))
        .unwrap_or(container_start);
    (track_duration + (video_start - container_start).max(0.0))
        .max(number(&format["duration"]).unwrap_or(0.0))
}

pub(super) async fn probe_overlay_duration(
    file: &Path,
    track_duration: f64,
) -> Result<f64, String> {
    let output = ffmpeg_utils::ffprobe_command()
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration,start_time:stream=codec_type,start_time",
            "-of",
            "json",
        ])
        .arg(file)
        .output()
        .await
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Failed to inspect overlay timeline: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let probe = serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
    Ok(overlay_timeline_duration(&probe, track_duration))
}

async fn encode_danmu_plan<R: ProgressReporterTrait>(
    reporter: Option<&R>,
    file: &Path,
    plan: DanmuRenderPlan,
    options: &Danmu2AssOptions,
    bitrate: Option<NonZeroU32>,
    directory: &Path,
) -> Result<PathBuf, String> {
    let filename = file
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Invalid file name: {}", file.display()))?;
    let output_path = file.with_file_name(format!("{}{filename}", constants::PREFIX_DANMAKU));
    let input = std::path::absolute(file).map_err(|error| error.to_string())?;
    let absolute_output = std::path::absolute(&output_path).map_err(|error| error.to_string())?;
    log::info!("Encode video danmu task start: {}", file.display());
    // A finite duration is necessary for the bounded overlay producer. Never
    // silently turn a failed probe into an indefinite rawvideo input.
    let metadata = ffmpeg_utils::extract_video_metadata(file)
        .await
        .map_err(|error| format!("Failed to inspect source video: {error}"))?;
    let fps = danmu_output_fps(metadata.fps);
    let encoder = hwaccel::get_x264_encoder().await;
    let graph = overlay_graph(fps, hwaccel::is_vaapi_encoder(encoder));
    let spec = OverlaySpec {
        input: native_overlay::RenderInput {
            documents: vec![native_overlay::AssSource {
                text: plan.ass,
                kind: native_overlay::SourceKind::Danmaku,
            }],
            images: image_events(&plan.images, options.opacity),
        },
        width: 1920,
        height: 1080,
        fps,
        start: 0.0,
        duration: probe_overlay_duration(file, metadata.duration).await?,
    };
    let mut command = ffmpeg_command();
    command
        .current_dir(directory)
        .args(["-nostdin", "-i"])
        .arg(&input);
    add_overlay_input(&mut command, &spec)?;
    command
        .arg("-filter_complex")
        .arg(graph)
        .args(["-map", "[outv]", "-map", "0:a?"]);
    hwaccel::apply_x264_encoder_only(&mut command, encoder);
    hwaccel::apply_x264_quality_args_with_bitrate(&mut command, encoder, bitrate);
    command
        .args(["-c:a", "copy", "-r", &fps.to_string(), "-y"])
        .arg(&absolute_output);
    let output_guard = TempArtifact::new(&absolute_output);
    run_overlay_command(
        command,
        spec,
        ProgressMode::stage("压制弹幕", ProgressTotal::FirstInput),
        "Encode video danmu",
        reporter,
    )
    .await?;
    output_guard.keep();
    // The caller's TempDir keeps assets alive through encoding and removes them
    // on error/cancellation. The FFmpeg factory uses kill_on_drop for the process.
    log::info!("Encode video danmu task end: {}", output_path.display());
    Ok(output_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emoji(url: &str) -> EmojiContent {
        EmojiContent {
            url: url.into(),
            width: Some(80),
            height: Some(80),
            emoticon_unique: None,
        }
    }

    fn entry(ts: i64, url: &str) -> DanmuEntry {
        DanmuEntry {
            ts,
            event_type: "danmu".into(),
            content: "[emoji]".into(),
            emoji_content: Some(emoji(url)),
            user_name: None,
            price: None,
            sc_duration: None,
        }
    }

    #[derive(Clone)]
    struct NoopReporter;

    #[async_trait::async_trait]
    impl ProgressReporterTrait for NoopReporter {
        async fn update(&self, _content: &str) {}
        async fn finish(&self, _success: bool, _message: &str) {}
    }

    #[test]
    fn filter_complex_file_option_follows_the_installed_ffmpeg() {
        assert_eq!(
            select_filter_complex_file_option("-filter_complex_script <filename>"),
            "-filter_complex_script"
        );
        assert_eq!(
            select_filter_complex_file_option("-filter_complex <graph>\n-filter_complex_threads"),
            "-/filter_complex"
        );
    }

    #[test]
    fn overlay_duration_covers_delayed_video_without_using_absolute_pts() {
        let delayed = serde_json::json!({
            "format": {"start_time":"0.0", "duration":"5.0"},
            "streams": [{"codec_type":"audio", "start_time":"0.0"},
                        {"codec_type":"video", "start_time":"2.0"}],
        });
        assert_eq!(overlay_timeline_duration(&delayed, 3.0), 5.0);
        let ts = serde_json::json!({
            "format": {"start_time":"10000.0", "duration":"3.0"},
            "streams": [{"codec_type":"video", "start_time":"10000.2"}],
        });
        assert!((overlay_timeline_duration(&ts, 3.0) - 3.2).abs() < 0.001);
        assert_eq!(overlay_timeline_duration(&serde_json::json!({}), 3.0), 3.0);
    }

    #[test]
    fn output_fps_preserves_high_rates_and_defaults_to_60() {
        for fps in [0.0, 30.0, 59.94, f64::NAN, f64::INFINITY] {
            assert_eq!(danmu_output_fps(fps), 60.0);
        }
        assert_eq!(danmu_output_fps(60.0), 60.0);
        assert_eq!(danmu_output_fps(120.0), 120.0);
    }

    #[test]
    fn image_urls_match_preview_security_policy() {
        for raw in [
            "http://i0.hdslb.com/a.png",
            " //i0.hdslb.com/a.png ",
            "https://i0.hdslb.com/a.png#fragment",
        ] {
            assert_eq!(
                image_url(&emoji(raw)).unwrap().as_str(),
                "https://i0.hdslb.com/a.png"
            );
        }
        assert!(image_url(&emoji("https://biliimg.com/a.webp")).is_some());
        for raw in [
            "",
            "file:///tmp/image.png",
            "data:image/png,xxx",
            "https://127.0.0.1/a.png",
            "https://hdslb.com.evil.test/a.png",
            "https://evilhdslb.com/a.png",
            "https://u:p@i0.hdslb.com/a.png",
            "https://i0.hdslb.com:8443/a.png",
        ] {
            assert!(image_url(&emoji(raw)).is_none(), "accepted {raw}");
        }
        assert!(!allowed_image_url(
            &Url::parse("https://localhost/image.png").unwrap()
        ));
    }

    #[test]
    fn image_footprints_match_preview_and_clamp_bad_dimensions() {
        let mut metadata = emoji("https://i0.hdslb.com/a.png");
        // A row high, as Bilibili draws emoticons.
        let row = image_height(36.0);
        assert!((row - 30.68).abs() < 0.01, "{row}");
        assert_eq!(image_size(&metadata, 36.0), (row, row));
        metadata.width = Some(160);
        assert_eq!(image_size(&metadata, 36.0), (row * 2.0, row));
        metadata.width = Some(u64::MAX);
        assert_eq!(image_size(&metadata, 36.0), (row * 3.0, row));
        metadata.width = Some(1);
        assert_eq!(image_size(&metadata, 36.0), (row * 0.25, row));
        metadata.height = Some(0);
        let large = image_height(48.0);
        assert_eq!(image_size(&metadata, 48.0), (large, large));
        metadata.height = None;
        assert_eq!(image_size(&metadata, 36.0), (row, row));
    }

    #[test]
    fn only_supported_image_payloads_are_decoded() {
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\n..."), Some("png"));
        assert_eq!(image_extension(b"RIFF1234WEBP..."), Some("webp"));
        assert_eq!(image_extension(b"GIF89a..."), Some("gif"));
        assert_eq!(image_extension(b"\xff\xd8\xff..."), Some("jpg"));
        assert_eq!(image_extension(b"#EXTM3U\nhttps://localhost/file"), None);
        assert_eq!(image_extension(b"<svg xmlns=...>"), None);
        assert_eq!(image_extension(b"RIFF1234WAVE"), None);
    }

    #[test]
    fn downloads_are_deduplicated_and_skip_non_danmu() {
        let mut sc = entry(0, "https://i0.hdslb.com/sc.png");
        sc.event_type = "super_chat".into();
        let urls = image_urls(&[
            entry(0, "http://i0.hdslb.com/a.png"),
            entry(1, "//i0.hdslb.com/a.png#fragment"),
            entry(-1, "https://i0.hdslb.com/before.png"),
            entry(2, "file:///tmp/local.png"),
            sc,
        ]);
        assert_eq!(
            urls,
            vec![Url::parse("https://i0.hdslb.com/a.png").unwrap()]
        );
    }

    #[test]
    fn text_only_and_vaapi_graphs_have_explicit_output_formats() {
        let text = overlay_graph(60.0, false);
        assert!(text.contains("fps=60[base]"));
        assert!(text.contains("[base][1:v]overlay=shortest=1:format=auto:alpha=straight"));
        assert!(!text.contains("ass="));
        assert!(
            !text.contains("setpts"),
            "do not rebase video without audio"
        );
        assert!(text.contains("format=yuv420p[outv]"));
        let hardware = overlay_graph(120.0, true);
        assert!(hardware.contains("fps=120[base]"));
        assert!(hardware.ends_with("format=nv12,hwupload[outv]\n"));
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg"]
    async fn an_image_decoded_before_is_not_downloaded_again() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let directory = tempfile::tempdir().unwrap();
        let png = directory.path().join("source.png");
        let made = ffmpeg_command()
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=8x8",
                "-frames:v",
                "1",
            ])
            .arg(&png)
            .output()
            .await
            .unwrap();
        assert!(made.status.success());
        let body = std::fs::read(&png).unwrap();
        std::fs::remove_file(&png).unwrap();
        // Answers one request only.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!(
            "http://{}/emoji.png",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(&body).await.unwrap();
        });
        let client = reqwest::Client::new();
        let first = download_image(&client, &url, directory.path(), 0)
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(
            first.file_name().unwrap().to_str().unwrap(),
            image_file_name(&url)
        );
        // Nothing answers now: it is found decoded.
        let again = download_image(&client, &url, directory.path(), 1)
            .await
            .unwrap();
        assert_eq!(again, first);
        let names: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, [image_file_name(&url)]);
    }

    #[tokio::test]
    async fn download_errors_and_oversized_responses_are_rejected_before_decode() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (response, expected) in [
            (
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_string(),
                "HTTP status",
            ),
            (
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                    MAX_IMAGE_BYTES + 1
                ),
                "download limit",
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\noops".to_string(),
                "unsupported image format",
            ),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url =
                Url::parse(&format!("http://{}/image", listener.local_addr().unwrap())).unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                assert!(socket.read(&mut request).await.unwrap() > 0);
                socket.write_all(response.as_bytes()).await.unwrap();
            });
            let directory = tempfile::tempdir().unwrap();
            let error = download_image(&reqwest::Client::new(), &url, directory.path(), 0)
                .await
                .unwrap_err();
            assert!(error.contains(expected), "unexpected error: {error}");
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn invalid_options_are_rejected_before_downloading() {
        let options = Danmu2AssOptions {
            font_size: f64::NAN,
            opacity: 0.8,
        };
        let error = encode_video_danmu(
            None::<&NoopReporter>,
            Path::new("missing.mp4"),
            &[],
            &options,
            None,
        )
        .await
        .unwrap_err();
        assert!(error.contains("Invalid danmaku"));
    }

    async fn run_ffmpeg(args: &[&str], output: &Path) {
        let result = ffmpeg_command()
            .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
            .args(args)
            .arg("-y")
            .arg(output)
            .output()
            .await
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    async fn rgb_frame(file: &Path, seconds: f64) -> Vec<u8> {
        let result = ffmpeg_command()
            .args(["-v", "error", "-ss", &seconds.to_string(), "-i"])
            .arg(file)
            .args([
                "-frames:v",
                "1",
                "-pix_fmt",
                "rgb24",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .output()
            .await
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout.len(), 1920 * 1080 * 3);
        result.stdout
    }

    fn red_bounds(frame: &[u8], top: usize, bottom: usize) -> Option<(usize, usize, usize, usize)> {
        let mut bounds = (usize::MAX, 0, usize::MAX, 0);
        for y in top..bottom {
            for x in 0..1920 {
                let i = (y * 1920 + x) * 3;
                let (r, g, b) = (frame[i], frame[i + 1], frame[i + 2]);
                if r > 40 && r.saturating_sub(g) > 30 && r.saturating_sub(b) > 30 {
                    bounds = (
                        bounds.0.min(x),
                        bounds.1.max(x),
                        bounds.2.min(y),
                        bounds.3.max(y),
                    );
                }
            }
        }
        (bounds.0 != usize::MAX).then_some(bounds)
    }

    /// Run explicitly with `cargo test ... ffmpeg_image_danmu_workflow -- --ignored`.
    #[tokio::test]
    #[ignore = "requires FFmpeg with libx264/libwebp and native Skia renderer"]
    async fn ffmpeg_image_danmu_workflow() {
        // Paths containing quotes, spaces and Unicode must never enter filter syntax.
        let directory = tempfile::Builder::new()
            .prefix("bili test's 图片-")
            .tempdir()
            .unwrap();
        let source_image = directory.path().join("source.png");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=red@0.5:s=40x10:d=0.1,format=rgba",
                "-frames:v",
                "1",
                "-update",
                "1",
            ],
            &source_image,
        )
        .await;
        let normalized = directory.path().join("normalized.png");
        decode_image(&source_image, &normalized).await.unwrap();
        for extension in ["jpg", "webp", "gif"] {
            let encoded = directory.path().join(format!("source.{extension}"));
            run_ffmpeg(
                &["-i", source_image.to_str().unwrap(), "-frames:v", "1"],
                &encoded,
            )
            .await;
            decode_image(
                &encoded,
                &directory.path().join(format!("decoded-{extension}.png")),
            )
            .await
            .unwrap();
        }
        let corrupt = directory.path().join("corrupt.png");
        std::fs::write(&corrupt, b"\x89PNG\r\n\x1a\ncorrupt").unwrap();
        assert!(
            decode_image(&corrupt, &directory.path().join("invalid.png"))
                .await
                .is_err()
        );

        let source = directory.path().join("source.mp4");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=320x180:r=30:d=30",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=30",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-c:a",
                "aac",
                "-shortest",
            ],
            &source,
        )
        .await;
        let options = Danmu2AssOptions::default();
        let mut entries = vec![entry(0, "image"), entry(0, "image"), entry(14000, "image")];
        let mut text = entry(0, "");
        text.content = "hello".into();
        text.emoji_content = None;
        entries.push(text);
        // Deliberately reserve a 2:1 box for a 4:1 image: object-contain must
        // preserve the image's actual aspect ratio and add transparent padding.
        // Each 162 px image enters at x = 1971 and travels 2183 px in 13.74 s;
        // the second, drawn 0.1 s later, goes in the row below the first.
        let plan = danmu_render_plan(&entries, &options, |entry| {
            entry.emoji_content.as_ref().map(|_| DanmuImage {
                path: normalized.clone(),
                width: 108.0,
                height: 54.0,
            })
        });
        let job = directory.path().join("job");
        std::fs::create_dir(&job).unwrap();
        let output = encode_danmu_plan(None::<&NoopReporter>, &source, plan, &options, None, &job)
            .await
            .unwrap();
        let metadata = ffmpeg_utils::extract_video_metadata(&output).await.unwrap();
        assert_eq!((metadata.width, metadata.height), (1920, 1080));
        assert_eq!(metadata.fps, 60.0);
        assert!((metadata.duration - 30.0).abs() < 0.1);
        let first = rgb_frame(&output, 4.0).await;
        let bounds = red_bounds(&first, 0, 80).expect("image missing");
        assert!(
            (bounds.0 as i64 - 1335).abs() < 4,
            "unexpected position: {bounds:?}"
        );
        assert!(
            (bounds.1 - bounds.0 + 1).abs_diff(162) < 4,
            "unexpected width: {bounds:?}"
        );
        assert!(
            (bounds.3 - bounds.2 + 1).abs_diff(40) < 4,
            "aspect ratio changed: {bounds:?}"
        );
        assert!(
            (bounds.2 as i64 - 20).abs() < 4,
            "image not centered in footprint: {bounds:?}"
        );
        assert!(
            red_bounds(&first, 81, 162).is_some(),
            "concurrent duplicate disappeared"
        );
        let center = (40 * 1920 + 1416) * 3;
        assert!(
            (90..115).contains(&first[center]),
            "alpha/opacity lost: {}",
            first[center]
        );
        let next = rgb_frame(&output, 4.0 + 1.0 / 60.0).await;
        assert!(
            red_bounds(&next, 0, 80).unwrap().0 < bounds.0,
            "60 FPS motion was duplicated"
        );
        let repeated = rgb_frame(&output, 15.0).await;
        assert!(
            red_bounds(&repeated, 0, 80).is_some(),
            "reused overlay disappeared"
        );
        assert!(
            red_bounds(&repeated, 81, 162).is_none(),
            "ended image remained visible"
        );
        let ended = rgb_frame(&output, 28.5).await;
        assert!(
            red_bounds(&ended, 0, 162).is_none(),
            "image remained after its event ended"
        );
        let probe = ffmpeg_utils::ffprobe_command()
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type",
                "-of",
                "json",
            ])
            .arg(&output)
            .output()
            .await
            .unwrap();
        let streams: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        assert!(streams["streams"]
            .as_array()
            .unwrap()
            .iter()
            .any(|stream| stream["codec_type"] == "audio"));
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg with libx264 and native Skia renderer"]
    async fn ffmpeg_preserves_delayed_video_and_shared_danmu_clock() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("delayed.mp4");
        run_ffmpeg(&[
            "-f", "lavfi", "-i",
            "color=c=blue:s=320x180:r=30:d=3,drawbox=x=0:y=0:w=160:h=180:color=red:t=fill:enable='gte(t,1)'",
            "-f", "lavfi", "-i", "sine=frequency=440:duration=5",
            "-vf", "setpts=PTS+2/TB", "-fps_mode", "passthrough",
            "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac",
        ], &source).await;
        let image = directory.path().join("image.png");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=green:s=40x40:d=0.1,format=rgba",
                "-frames:v",
                "1",
                "-update",
                "1",
            ],
            &image,
        )
        .await;
        let options = Danmu2AssOptions {
            font_size: 36.0,
            opacity: 1.0,
        };
        let plan = danmu_render_plan(&[entry(0, "image")], &options, |_| {
            Some(DanmuImage {
                path: image.clone(),
                width: 54.0,
                height: 54.0,
            })
        });
        let output = encode_danmu_plan(
            None::<&NoopReporter>,
            &source,
            plan,
            &options,
            None,
            directory.path(),
        )
        .await
        .unwrap();
        // Source video starts at media t=2, and changes from blue to half-red at
        // t=3. Video-only rebasing would move that change to t=1, out of sync
        // with the copied audio. CFR may duplicate initial frames, so inspect
        // actual content rather than only the output stream's first packet.
        let before = rgb_frame(&output, 2.5).await;
        let after = rgb_frame(&output, 3.5).await;
        let pixel = (500 * 1920 + 400) * 3;
        assert!(before[pixel + 2] > 200 && before[pixel] < 50);
        assert!(after[pixel] > 200 && after[pixel + 2] < 50);
        // The t=0 image is already 4 seconds into motion at media t=4, not
        // restarted at the first decoded video frame (media t=2): its 81 px
        // travel 2102 px in 13.76 s from x = 1971.
        let frame = rgb_frame(&output, 4.0).await;
        let pixel = (40 * 1920 + 1400) * 3;
        assert!(frame[pixel + 1] > 80 && frame[pixel] < 50 && frame[pixel + 2] < 50);
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg with libx264 and native Skia renderer"]
    async fn ffmpeg_text_fallback_without_audio() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("silent.mp4");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=320x180:r=30:d=5",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
            ],
            &source,
        )
        .await;
        let entries = [entry(0, "file:///not-allowed.png")];
        let output = encode_video_danmu(
            None::<&NoopReporter>,
            &source,
            &entries,
            &Danmu2AssOptions::default(),
            // Also exercises a chosen bitrate in place of the default quality.
            NonZeroU32::new(2000),
        )
        .await
        .unwrap();
        let frame = rgb_frame(&output, 4.0).await;
        assert!(
            frame.iter().any(|value| *value > 70),
            "fallback text was not burned in"
        );
        let metadata = ffmpeg_utils::extract_video_metadata(&output).await.unwrap();
        assert_eq!(metadata.fps, 60.0);
        assert!((metadata.duration - 5.0).abs() < 0.1);
    }
}
