use std::path::{Path, PathBuf};

use m3u8_rs::{Map, MediaPlaylist};
use tokio::io::AsyncWriteExt;

use crate::progress::progress_reporter::ProgressReporterTrait;

#[cfg(target_os = "windows")]
use crate::ffmpeg::CREATE_NO_WINDOW;
#[cfg(target_os = "windows")]
#[allow(unused_imports)]
use std::os::windows::process::CommandExt;

use super::Range;

pub async fn clip_multiple_from_playlist(
    reporter: Option<&impl ProgressReporterTrait>,
    playlist_path: &Path,
    output_path: &Path,
    ranges: &[Range],
    transition: Option<&str>,
) -> Result<(), String> {
    let mut to_remove = Vec::new();
    for (i, range) in ranges.iter().enumerate() {
        let video_path = output_path.with_extension(format!("{}.mp4", i));
        if let Err(e) =
            clip_from_playlist(reporter, playlist_path, &video_path, Some(range.clone())).await
        {
            log::error!("Failed to generate playlist video: {e}");
            // clean up to_remove
            for path in to_remove {
                let _ = tokio::fs::remove_file(path).await;
            }
            return Err(e);
        }
        to_remove.push(video_path.clone());
    }
    super::general::concat_videos_with_transition(reporter, &to_remove, output_path, transition)
        .await?;
    // clean up to_remove
    for path in to_remove {
        let _ = tokio::fs::remove_file(path).await;
    }
    Ok(())
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

    // transcode copy to fix timestamp
    {
        let tmp_output_path = output_path.with_extension("tmp.mp4");
        super::transcode(reporter, output_path, &tmp_output_path, true).await?;

        // remove original file
        let _ = tokio::fs::remove_file(output_path).await;
        // rename tmp_output_path to output_path
        let _ = tokio::fs::rename(tmp_output_path, output_path).await;
    }

    // trim for precised duration
    if let (Some(start_offset), Some(range)) = (start_offset, range.as_ref()) {
        let tmp_output_path = output_path.with_extension("tmp.mp4");
        super::trim_video(
            reporter,
            output_path,
            &tmp_output_path,
            start_offset,
            range.duration(),
        )
        .await?;

        // remove original file
        let _ = tokio::fs::remove_file(output_path).await;
        // rename tmp_output_path to output_path
        let _ = tokio::fs::rename(tmp_output_path, output_path).await;
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

pub async fn concat_playlists_to_video(
    reporter: Option<&impl ProgressReporterTrait>,
    playlists: &[&Path],
    danmu_ass_files: Vec<Option<PathBuf>>,
    output_path: &Path,
) -> Result<(), String> {
    let mut to_remove = Vec::new();
    let mut segments = Vec::new();
    for (i, playlist) in playlists.iter().enumerate() {
        let mut video_path = output_path.with_extension(format!("{}.mp4", i));
        if let Err(e) = clip_from_playlist(reporter, playlist, &video_path, None).await {
            log::error!("Failed to generate playlist video: {e}");
            continue;
        }
        to_remove.push(video_path.clone());
        if let Some(danmu_ass_file) = &danmu_ass_files[i] {
            video_path = super::encode_video_danmu(reporter, &video_path, danmu_ass_file).await?;
            to_remove.push(video_path.clone());
        }
        segments.push(video_path);
    }

    super::general::concat_videos(reporter, &segments, output_path).await?;

    // clean up segments
    for segment in to_remove {
        let _ = tokio::fs::remove_file(segment).await;
    }

    Ok(())
}
