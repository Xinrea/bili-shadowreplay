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

    let first_segment = playlist
        .segments
        .first()
        .ok_or_else(|| "Playlist contains no segments".to_string())?;
    let mut header_url = first_segment
        .unknown_tags
        .iter()
        .find(|t| t.tag == "X-MAP")
        .and_then(|tag| tag.rest.as_deref())
        .and_then(parse_map_uri);
    if header_url.is_none() {
        // map: Some(Map { uri: "h1758725308.m4s"
        if let Some(Map { uri, .. }) = &first_segment.map {
            header_url = Some(uri.clone());
        }
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
        if let Some(header_url) = header_url {
            let header_data = tokio::fs::read(playlist_folder.join(header_url))
                .await
                .map_err(|e| format!("Failed to read header file: {}", e))?;
            file.write_all(&header_data)
                .await
                .map_err(|e| format!("Failed to write header file: {}", e))?;
        }
        for s in segments {
            // read segment
            let uri = s.uri.split('?').next().unwrap_or(&s.uri);
            let segment_file_path = playlist_folder.join(uri);
            let segment_data = tokio::fs::read(&segment_file_path)
                .await
                .map_err(|e| format!("Failed to read segment file: {}", e))?;
            // append segment data to clip_file
            file.write_all(&segment_data)
                .await
                .map_err(|e| format!("Failed to write segment file: {}", e))?;
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
}
