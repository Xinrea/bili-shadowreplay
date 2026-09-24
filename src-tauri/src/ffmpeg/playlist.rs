use std::path::{Path, PathBuf};

use m3u8_rs::{Map, MediaPlaylist};
use tempfile::{tempdir_in, Builder};
use tokio::io::AsyncWriteExt;

use crate::progress::progress_reporter::ProgressReporterTrait;

use super::Range;

pub async fn clip_multiple_from_playlist(
    reporter: Option<&impl ProgressReporterTrait>,
    playlist_path: &Path,
    output_path: &Path,
    ranges: &[Range],
    transition: Option<&str>,
) -> Result<(), String> {
    let output_dir = output_path
        .parent()
        .ok_or_else(|| format!("Output path has no parent: {}", output_path.display()))?;
    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create output directory: {e}"))?;
    let temp_dir = tempdir_in(output_dir)
        .map_err(|e| format!("Failed to create playlist clip directory: {e}"))?;
    let mut clips = Vec::with_capacity(ranges.len());

    for (i, range) in ranges.iter().enumerate() {
        let video_path = temp_dir.path().join(format!("clip-{i}.mp4"));
        clip_from_playlist(reporter, playlist_path, &video_path, Some(range.clone())).await?;
        clips.push(video_path);
    }

    super::general::concat_videos_with_transition(reporter, &clips, output_path, transition).await
}

pub async fn clip_from_playlist(
    reporter: Option<&impl ProgressReporterTrait>,
    playlist_path: &Path,
    output_path: &Path,
    range: Option<Range>,
) -> Result<(), String> {
    let playlist_bytes = tokio::fs::read(playlist_path)
        .await
        .map_err(|e| format!("Failed to read playlist '{}': {e}", playlist_path.display()))?;
    let playlist = parse_media_playlist(&playlist_bytes, playlist_path)?;
    let mut start_offset = None;
    let mut segments = Vec::new();
    if let Some(range) = &range {
        let mut duration = 0.0;
        for s in playlist.segments.clone() {
            if range.is_in(duration) || range.is_in(duration + s.duration as f64) {
                segments.push(s.clone());
                if start_offset.is_none() {
                    start_offset = Some(range.start - duration);
                }
            }
            duration += s.duration as f64;
        }
    } else {
        segments = playlist.segments.clone();
    }

    if segments.is_empty() {
        return Err("No segments found".to_string());
    }

    // write all segments to clip_file
    {
        let playlist_folder = playlist_path.parent().unwrap_or_else(|| Path::new("."));
        let output_folder = output_path.parent().unwrap_or_else(|| Path::new("."));
        if !output_folder.exists() {
            std::fs::create_dir_all(output_folder).map_err(|e| {
                format!(
                    "Failed to create output folder '{}': {e}",
                    output_folder.display()
                )
            })?;
        }
        let mut file = tokio::fs::File::create(&output_path)
            .await
            .map_err(|e| format!("Failed to create output file: {}", e))?;

        // `#EXT-X-MAP` applies forward until the next map. A range clip may start
        // after the map tag, so discover the init that covers the first selected
        // segment by scanning from the playlist head.
        let mut active_map = map_uri_before_segment(&playlist, &segments[0].uri);
        if let Some(header_url) = active_map.as_ref() {
            write_playlist_file(&mut file, playlist_folder, header_url).await?;
        }

        for s in segments {
            if let Some(Map { uri, .. }) = &s.map {
                if active_map.as_deref() != Some(uri.as_str()) {
                    write_playlist_file(&mut file, playlist_folder, uri).await?;
                    active_map = Some(uri.clone());
                }
            } else if let Some(uri) = s
                .unknown_tags
                .iter()
                .find(|t| t.tag == "X-MAP")
                .and_then(|tag| tag.rest.as_deref())
                .and_then(parse_map_uri)
            {
                if active_map.as_deref() != Some(uri.as_str()) {
                    write_playlist_file(&mut file, playlist_folder, &uri).await?;
                    active_map = Some(uri);
                }
            }

            // read segment
            let uri = s.uri.split('?').next().unwrap_or(&s.uri);
            write_playlist_file(&mut file, playlist_folder, uri).await?;
        }
        file.flush()
            .await
            .map_err(|e| format!("Failed to flush file: {}", e))?;
    }

    // Remux into an RAII-managed sibling before replacing the assembled file.
    // A fixed `.tmp.mp4` could collide with another job and leaked on errors.
    {
        let tmp_output_path = temporary_output_path(output_path)?;
        super::transcode(reporter, output_path, &tmp_output_path, true).await?;
        replace_output(output_path, tmp_output_path).await?;
    }

    // Trim for the precise requested duration.
    if let (Some(start_offset), Some(range)) = (start_offset, range.as_ref()) {
        let tmp_output_path = temporary_output_path(output_path)?;
        super::trim_video(
            reporter,
            output_path,
            &tmp_output_path,
            start_offset,
            range.duration(),
        )
        .await?;
        replace_output(output_path, tmp_output_path).await?;
    }

    Ok(())
}

fn parse_media_playlist(bytes: &[u8], playlist_path: &Path) -> Result<MediaPlaylist, String> {
    m3u8_rs::parse_media_playlist(bytes)
        .map(|(_, playlist)| playlist)
        .map_err(|_| {
            let input_context = if bytes.is_empty() {
                "input is empty"
            } else if bytes.iter().all(|byte| *byte == 0) {
                "input is zero-filled"
            } else {
                "invalid playlist syntax"
            };
            format!(
                "Failed to parse media playlist '{}': {input_context} ({} bytes)",
                playlist_path.display(),
                bytes.len()
            )
        })
}

fn parse_map_uri(rest: &str) -> Option<String> {
    rest.split_once('=').and_then(|(_, value)| {
        let unescaped = value.trim().replace("\\\"", "\"");
        let uri = unescaped.trim_matches('"');
        (!uri.is_empty()).then(|| uri.to_string())
    })
}

fn segment_map_uri(segment: &m3u8_rs::MediaSegment) -> Option<String> {
    segment.map.as_ref().map(|map| map.uri.clone()).or_else(|| {
        segment
            .unknown_tags
            .iter()
            .find(|tag| tag.tag == "X-MAP")
            .and_then(|tag| tag.rest.as_deref())
            .and_then(parse_map_uri)
    })
}

/// Find the `#EXT-X-MAP` that applies to `segment_uri` by walking the playlist
/// from the start, the same way HLS players inherit init sections.
fn map_uri_before_segment(playlist: &MediaPlaylist, segment_uri: &str) -> Option<String> {
    let mut active = None;
    for segment in &playlist.segments {
        if let Some(uri) = segment_map_uri(segment) {
            active = Some(uri);
        }
        if segment.uri == segment_uri {
            return active;
        }
    }
    active
}

async fn write_playlist_file(
    file: &mut tokio::fs::File,
    playlist_folder: &Path,
    relative_uri: &str,
) -> Result<(), String> {
    let path = playlist_folder.join(relative_uri);
    let data = tokio::fs::read(&path)
        .await
        .map_err(|e| format!("Failed to read '{}': {e}", path.display()))?;
    file.write_all(&data)
        .await
        .map_err(|e| format!("Failed to write '{}': {e}", path.display()))
}

fn temporary_output_path(output_path: &Path) -> Result<tempfile::TempPath, String> {
    let output_dir = output_path
        .parent()
        .ok_or_else(|| format!("Output path has no parent: {}", output_path.display()))?;
    Builder::new()
        .prefix(".bili-ffmpeg-")
        .suffix(".mp4")
        .tempfile_in(output_dir)
        .map(|file| file.into_temp_path())
        .map_err(|e| format!("Failed to create temporary output: {e}"))
}

async fn replace_output(
    output_path: &Path,
    temporary_path: tempfile::TempPath,
) -> Result<(), String> {
    match tokio::fs::remove_file(output_path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Failed to remove original output '{}': {error}",
                output_path.display()
            ));
        }
    }
    tokio::fs::rename(&*temporary_path, output_path)
        .await
        .map_err(|e| {
            format!(
                "Failed to replace '{}' with temporary output: {e}",
                output_path.display()
            )
        })?;
    // The file now lives at `output_path`. Disable TempPath cleanup so Drop
    // does not try to unlink a path that was renamed away.
    let _ = temporary_path.keep();
    Ok(())
}

pub async fn concat_playlists_to_video(
    reporter: Option<&impl ProgressReporterTrait>,
    playlists: &[&Path],
    danmu_ass_files: Vec<Option<PathBuf>>,
    output_path: &Path,
) -> Result<(), String> {
    let output_dir = output_path
        .parent()
        .ok_or_else(|| format!("Output path has no parent: {}", output_path.display()))?;
    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create output directory: {e}"))?;
    let temp_dir = tempdir_in(output_dir)
        .map_err(|e| format!("Failed to create playlist concat directory: {e}"))?;
    let mut segments = Vec::new();

    for (i, playlist) in playlists.iter().enumerate() {
        let mut video_path = temp_dir.path().join(format!("playlist-{i}.mp4"));
        if let Err(e) = clip_from_playlist(reporter, playlist, &video_path, None).await {
            log::error!("Failed to generate playlist video: {e}");
            continue;
        }
        if let Some(danmu_ass_file) = danmu_ass_files.get(i).and_then(Option::as_ref) {
            video_path = super::encode_video_danmu(reporter, &video_path, danmu_ass_file).await?;
        }
        segments.push(video_path);
    }

    super::general::concat_videos(reporter, &segments, output_path).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_filled_playlist_without_panicking() {
        let path = Path::new("recordings/playlist.m3u8");
        let result = parse_media_playlist(&vec![0; 1024], path);

        assert_eq!(
            result.unwrap_err(),
            "Failed to parse media playlist 'recordings/playlist.m3u8': input is zero-filled (1024 bytes)"
        );
    }

    #[test]
    fn reports_empty_playlist_without_exposing_content() {
        let result = parse_media_playlist(&[], Path::new("empty.m3u8"));

        assert_eq!(
            result.unwrap_err(),
            "Failed to parse media playlist 'empty.m3u8': input is empty (0 bytes)"
        );
    }

    #[test]
    fn reports_invalid_playlist_without_exposing_content() {
        let result = parse_media_playlist(
            b"sensitive invalid playlist content",
            Path::new("invalid.m3u8"),
        );

        let error = result.unwrap_err();
        assert_eq!(
            error,
            "Failed to parse media playlist 'invalid.m3u8': invalid playlist syntax (34 bytes)"
        );
        assert!(!error.contains("sensitive"));
    }

    #[test]
    fn parses_map_uri() {
        assert_eq!(
            parse_map_uri(r#"URI=\"header.m4s\""#),
            Some("header.m4s".to_string())
        );
        assert_eq!(parse_map_uri("malformed"), None);
    }

    #[test]
    fn map_uri_before_segment_inherits_the_latest_map() {
        let playlist = r#"#EXTM3U
#EXT-X-TARGETDURATION:2
#EXT-X-MAP:URI="init-a.mp4"
#EXTINF:2.0,
seg-1.mp4
#EXTINF:2.0,
seg-2.mp4
#EXT-X-MAP:URI="init-b.mp4"
#EXTINF:2.0,
seg-3.mp4
"#;
        let (_, parsed) = m3u8_rs::parse_media_playlist(playlist.as_bytes()).unwrap();
        assert_eq!(
            map_uri_before_segment(&parsed, "seg-2.mp4").as_deref(),
            Some("init-a.mp4")
        );
        assert_eq!(
            map_uri_before_segment(&parsed, "seg-3.mp4").as_deref(),
            Some("init-b.mp4")
        );
    }
}
