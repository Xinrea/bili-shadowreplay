use std::path::Path;

use m3u8_rs::Map;
use tokio::io::AsyncWriteExt;

use crate::{
    ffmpeg::{ffmpeg_path, general::handle_ffmpeg_process},
    progress::progress_reporter::ProgressReporterTrait,
};

use super::Range;

pub async fn cache_playlist(reporter: Option<&impl ProgressReporterTrait>, playlist_url: &str, work_dir: &Path) -> Result<(), String> {
    // ffmpeg -i "http://example.com/live/stream.m3u8" \
    //   -timeout 10 \
    //   -reconnect 1 -reconnect_streamed 1 -reconnect_delay_max 10 \
    //   -hls_list_size 0 \
    //   -hls_time 10 \
    //   -hls_flags append_list+program_date_time \
    //   -hls_segment_filename "cache_%Y%m%d_%H%M%S_%03d.ts" \
    //   -strftime 1 \
    //   -c copy \
    //   playlist.m3u8

    let mut ffmpeg_process = tokio::process::Command::new(ffmpeg_path());
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);

    // set work_dir
    ffmpeg_process.current_dir(work_dir);

    ffmpeg_process
        .args(["-reconnect", "0"])
        .args(["-reconnect_streamed", "0"])
        .args(["-rw_timeout", "5000000"])
        .args(["-i", playlist_url])
        .args(["-hls_list_size", "0"])
        .args(["-hls_time", "5"])
        .args(["-hls_flags", "append_list+program_date_time"])
        .args(["-hls_segment_filename", "%Y%m%d_%H%M%S.ts"])
        .args(["-strftime", "1"])
        .args(["-c", "copy"])
        .args(["-progress", "pipe:2"])
        .args(["-y", "playlist.m3u8"]);

    handle_ffmpeg_process(
        reporter,
        &mut ffmpeg_process,
    )
    .await?;

    Ok(())
}

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
