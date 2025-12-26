use std::path::{Path, PathBuf};

use m3u8_rs::Map;
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
    super::general::concat_videos(reporter, &to_remove, output_path).await?;
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
    let (_, playlist) = m3u8_rs::parse_media_playlist(
        &tokio::fs::read(playlist_path)
            .await
            .map_err(|e| e.to_string())?,
    )
    .unwrap();
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

    let first_segment = playlist.segments.first().unwrap().clone();
    let mut header_url = first_segment
        .unknown_tags
        .iter()
        .find(|t| t.tag == "X-MAP")
        .map(|t| {
            let rest = t.rest.clone().unwrap();
            rest.split('=').nth(1).unwrap().replace("\\\"", "")
        });
    if header_url.is_none() {
        // map: Some(Map { uri: "h1758725308.m4s"
        if let Some(Map { uri, .. }) = &first_segment.map {
            header_url = Some(uri.clone());
        }
    }

    // write all segments to clip_file
    {
        let playlist_folder = playlist_path.parent().unwrap();
        let output_folder = output_path.parent().unwrap();
        if !output_folder.exists() {
            std::fs::create_dir_all(output_folder).unwrap();
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
    if let Some(start_offset) = start_offset {
        let tmp_output_path = output_path.with_extension("tmp.mp4");
        super::trim_video(
            reporter,
            output_path,
            &tmp_output_path,
            start_offset,
            range.as_ref().unwrap().duration(),
        )
        .await?;

        // remove original file
        let _ = tokio::fs::remove_file(output_path).await;
        // rename tmp_output_path to output_path
        let _ = tokio::fs::rename(tmp_output_path, output_path).await;
    }

    Ok(())
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
