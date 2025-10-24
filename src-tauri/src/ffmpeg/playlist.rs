use std::path::Path;

use m3u8_rs::Map;
use tokio::io::AsyncWriteExt;

use crate::progress::progress_reporter::ProgressReporterTrait;

#[cfg(target_os = "windows")]
use crate::ffmpeg::CREATE_NO_WINDOW;
#[cfg(target_os = "windows")]
#[allow(unused_imports)]
use std::os::windows::process::CommandExt;

use super::Range;

pub async fn playlist_to_video(
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
        let mut file = tokio::fs::File::create(&output_path).await.unwrap();
        if let Some(header_url) = header_url {
            let header_data = tokio::fs::read(playlist_folder.join(header_url))
                .await
                .unwrap();
            file.write_all(&header_data).await.unwrap();
        }
        for s in segments {
            // read segment
            let segment_file_path = playlist_folder.join(s.uri);
            let segment_data = tokio::fs::read(&segment_file_path).await.unwrap();
            // append segment data to clip_file
            file.write_all(&segment_data).await.unwrap();
        }
        file.flush().await.unwrap();
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

pub async fn playlists_to_video(
    reporter: Option<&impl ProgressReporterTrait>,
    playlists: &[&Path],
    output_path: &Path,
) -> Result<(), String> {
    let mut segments = Vec::new();
    for (i, playlist) in playlists.iter().enumerate() {
        let video_path = output_path.with_extension(format!("{}.mp4", i));
        playlist_to_video(reporter, playlist, &video_path, None).await?;
        segments.push(video_path);
    }

    super::general::concat_videos(reporter, &segments, output_path).await?;

    // clean up segments
    for segment in segments {
        let _ = tokio::fs::remove_file(segment).await;
    }

    Ok(())
}
