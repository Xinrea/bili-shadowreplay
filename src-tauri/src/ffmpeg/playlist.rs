use std::{num::NonZeroU32, path::Path};

use m3u8_rs::{Map, MediaPlaylist};
use tempfile::{tempdir_in, Builder};
use tokio::io::AsyncWriteExt;

use crate::progress::progress_reporter::ProgressReporterTrait;

use super::Range;

/// Cut `ranges` from a playlist and join them into `output_path`, returning the
/// recording ranges the output covers.
///
/// A stream-copied cut keeps the packets up to the keyframe before its start
/// behind an edit list and runs a little past its end. Without encoding options
/// or a transition, the pieces are joined by stream copy. Explicit encoding
/// applies those edit lists before joining. A single range is only remuxed,
/// so it is returned as asked for the caller to measure. The join targets
/// `bitrate` (kbps) when given.
pub async fn clip_multiple_from_playlist(
    reporter: Option<&impl ProgressReporterTrait>,
    playlist_path: &Path,
    output_path: &Path,
    ranges: &[Range],
    transition: Option<&str>,
    bitrate: Option<NonZeroU32>,
) -> Result<Vec<Range>, String> {
    clip_multiple_with_settings(
        reporter,
        playlist_path,
        output_path,
        ranges,
        transition,
        bitrate,
        None,
    )
    .await
}

pub async fn clip_multiple_with_settings(
    reporter: Option<&impl ProgressReporterTrait>,
    playlist_path: &Path,
    output_path: &Path,
    ranges: &[Range],
    transition: Option<&str>,
    bitrate: Option<NonZeroU32>,
    settings: Option<&crate::project::RenderSettings>,
) -> Result<Vec<Range>, String> {
    if let Some(settings) = settings {
        settings.validate()?;
    }
    let output_dir = output_path
        .parent()
        .ok_or_else(|| format!("Output path has no parent: {}", output_path.display()))?;
    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create output directory: {e}"))?;
    let temp_dir = tempdir_in(output_dir)
        .map_err(|e| format!("Failed to create playlist clip directory: {e}"))?;
    let count = ranges.len();
    let copy = settings.is_none()
        && bitrate.is_none()
        && (transition.is_none() || transition == Some("none"));
    let mut clips = Vec::with_capacity(count);
    let mut covered = Vec::with_capacity(count);

    for (i, range) in ranges.iter().enumerate() {
        let video_path = temp_dir.path().join(format!("clip-{i}.mp4"));
        let stage = if count > 1 {
            format!("提取片段 {}/{count}", i + 1)
        } else {
            "提取片段".to_string()
        };
        let extracting = reporter.map(|reporter| reporter.stage(&stage));
        clip_from_playlist(
            extracting.as_ref(),
            playlist_path,
            &video_path,
            Some(range.clone()),
        )
        .await?;
        let duration = match ffmpeg_utils::extract_video_metadata(&video_path).await {
            Ok(metadata) if count > 1 && metadata.duration > 0.0 => metadata.duration,
            _ => range.duration(),
        };
        covered.push(if copy && count > 1 {
            // Each copied range can include the preceding keyframe. Keep
            // external danmaku aligned with those retained packets.
            Range {
                start: (range.end - duration).max(0.0),
                end: range.end,
            }
        } else {
            Range {
                start: range.start,
                end: range.start + duration,
            }
        });
        clips.push(video_path);
    }

    // A single clip is only remuxed here, so it stays part of extraction.
    // Defer its explicit encode to the material's burn/fix/final stage rather
    // than re-encoding before we know whether danmaku needs burning.
    let merging = reporter.map(|reporter| {
        reporter.stage(if count > 1 {
            "合并片段"
        } else {
            "提取片段"
        })
    });
    if copy {
        super::general::concat_copy_videos(merging.as_ref(), &clips, output_path).await?;
    } else {
        super::general::concat_videos_with_settings(
            merging.as_ref(),
            &clips,
            output_path,
            transition,
            bitrate,
            settings.filter(|_| count > 1),
        )
        .await?;
    }
    Ok(covered)
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
        super::transcode(reporter, output_path, &tmp_output_path, true, None).await?;
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

/// Concatenate `playlists` into `output_path`, burning each one's danmaku
/// track when it has one. Returns whether any danmaku made it into the video;
/// a playlist that fails to cut is left out.
pub async fn concat_playlists_to_video(
    reporter: Option<&impl ProgressReporterTrait>,
    playlists: &[&Path],
    danmu_tracks: Vec<Option<Vec<recorder::danmu::DanmuEntry>>>,
    danmu_options: &crate::danmu2ass::Danmu2AssOptions,
    output_path: &Path,
    settings: Option<&crate::project::RenderSettings>,
) -> Result<bool, String> {
    if let Some(settings) = settings {
        settings.validate()?;
    }
    let output_dir = output_path
        .parent()
        .ok_or_else(|| format!("Output path has no parent: {}", output_path.display()))?;
    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create output directory: {e}"))?;
    let temp_dir = tempdir_in(output_dir)
        .map_err(|e| format!("Failed to create playlist concat directory: {e}"))?;
    let mut segments = Vec::new();
    let mut burned = false;
    // If any source needs burning, encode all sources through the same overlay
    // path (including empty tracks) so matching outputs can be joined by copy.
    // Otherwise defer encoding to the final join, including single-input jobs.
    let encode_segments = settings.is_some()
        && danmu_tracks
            .iter()
            .flatten()
            .flatten()
            .any(recorder::danmu::DanmuEntry::is_danmu);

    for (i, playlist) in playlists.iter().enumerate() {
        let mut video_path = temp_dir.path().join(format!("playlist-{i}.mp4"));
        if let Err(e) = clip_from_playlist(reporter, playlist, &video_path, None).await {
            log::error!("Failed to generate playlist video: {e}");
            continue;
        }
        let track = danmu_tracks.get(i).and_then(Option::as_ref);
        if encode_segments || (settings.is_none() && track.is_some()) {
            let danmus = track.map(Vec::as_slice).unwrap_or_default();
            video_path = match settings {
                Some(settings) => {
                    super::encode_video_danmu_with_settings(
                        reporter,
                        &video_path,
                        danmus,
                        danmu_options,
                        None,
                        Some(settings),
                    )
                    .await?
                }
                None => {
                    super::encode_video_danmu(reporter, &video_path, danmus, danmu_options, None)
                        .await?
                }
            };
            burned |= danmus.iter().any(recorder::danmu::DanmuEntry::is_danmu);
        }
        segments.push(video_path);
    }

    match settings {
        Some(settings) if encode_segments => {
            super::general::concat_encoded_videos(reporter, &segments, output_path, settings)
                .await?;
        }
        Some(settings) => {
            super::general::concat_videos_with_settings(
                reporter,
                &segments,
                output_path,
                None,
                None,
                Some(settings),
            )
            .await?;
        }
        None if burned => {
            super::general::concat_videos(reporter, &segments, output_path).await?;
        }
        None => super::general::concat_copy_videos(reporter, &segments, output_path).await?,
    }
    Ok(burned)
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

    /// A recording with a keyframe every 2 s, cut into 2 s HLS segments.
    async fn create_test_playlist(dir: &Path, duration: f64) -> std::path::PathBuf {
        create_test_playlist_with_size(dir, duration, "320x180").await
    }

    async fn create_test_playlist_with_size(
        dir: &Path,
        duration: f64,
        size: &str,
    ) -> std::path::PathBuf {
        let playlist = dir.join("playlist.m3u8");
        let output = ffmpeg_utils::ffmpeg_command()
            .args([
                "-f",
                "lavfi",
                "-i",
                &format!("color=c=blue:s={size}:r=30:d={duration}"),
                "-f",
                "lavfi",
                "-i",
                &format!("sine=d={duration}"),
                "-c:v",
                "libx264",
                // B-frames, as live streams have, make a copied cut run past its end.
                "-bf",
                "3",
                "-g",
                "60",
                "-keyint_min",
                "60",
                "-sc_threshold",
                "0",
                "-c:a",
                "aac",
                "-shortest",
                "-f",
                "hls",
                "-hls_time",
                "2",
                "-hls_playlist_type",
                "vod",
                "-y",
            ])
            .arg(&playlist)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        playlist
    }

    async fn packet_hashes(path: &Path) -> Vec<String> {
        let output = ffmpeg_utils::ffprobe_command()
            .args([
                "-v",
                "error",
                "-show_packets",
                "-show_data_hash",
                "sha256",
                "-show_entries",
                "packet=data_hash",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        let packets: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        packets["packets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|packet| packet["data_hash"].as_str().unwrap().to_string())
            .collect()
    }

    async fn video_timeline(path: &Path) -> (serde_json::Value, Vec<serde_json::Value>) {
        let output = ffmpeg_utils::ffprobe_command()
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_streams",
                "-show_frames",
                "-show_entries",
                "stream=codec_name,width,height,pix_fmt,has_b_frames,time_base,r_frame_rate,avg_frame_rate,start_pts,start_time,duration_ts,duration,nb_frames:frame=pts,best_effort_timestamp,duration,pkt_duration",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let streams = value["streams"].as_array().unwrap();
        assert_eq!(streams.len(), 1);
        let frames = value["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|frame| {
                serde_json::json!({
                    "pts": frame["pts"],
                    "best_effort_timestamp": frame["best_effort_timestamp"],
                    "duration": frame["duration"],
                    "pkt_duration": frame["pkt_duration"],
                })
            })
            .collect();
        (streams[0].clone(), frames)
    }

    async fn join_cli_reference(clips: &[std::path::PathBuf], output: &Path) {
        let list = output.with_extension("txt");
        std::fs::write(
            &list,
            clips
                .iter()
                .map(|clip| format!("file '{}'\n", clip.display()))
                .collect::<String>(),
        )
        .unwrap();
        let result = ffmpeg_utils::ffmpeg_command()
            .args(["-f", "concat", "-safe", "0", "-i"])
            .arg(&list)
            .args(["-c", "copy", "-y"])
            .arg(output)
            .output()
            .await
            .unwrap();
        assert!(
            result.status.success(),
            "CLI copy reference failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[tokio::test]
    async fn copied_ranges_report_the_recording_the_output_covers() {
        let dir = tempfile::tempdir().unwrap();
        let playlist = create_test_playlist(dir.path(), 30.0).await;
        let output = dir.path().join("joined.mp4");
        // Neither range starts on a keyframe.
        let ranges = [
            Range {
                start: 7.3,
                end: 10.3,
            },
            Range {
                start: 21.2,
                end: 24.2,
            },
        ];
        let covered = clip_multiple_from_playlist(
            None::<&crate::progress::progress_reporter::ProgressReporter>,
            &playlist,
            &output,
            &ranges,
            None,
            None,
        )
        .await
        .unwrap();

        // Keep the returned recording boundaries tied to each measured cut.
        // The copy path deliberately retains concat demuxer/edit-list behavior.
        // Compare with CLI stream copy rather than a decoded concat filter.
        let mut reference_cuts = Vec::new();
        for (index, (covered, asked)) in covered.iter().zip(&ranges).enumerate() {
            let cut = dir.path().join(format!("reference-cut-{index}.mp4"));
            clip_from_playlist(
                None::<&crate::progress::progress_reporter::ProgressReporter>,
                &playlist,
                &cut,
                Some(asked.clone()),
            )
            .await
            .unwrap();
            let metadata = ffmpeg_utils::extract_video_metadata(&cut).await.unwrap();
            assert_eq!(covered.start, (asked.end - metadata.duration).max(0.0));
            assert_eq!(covered.end, asked.end);
            reference_cuts.push(cut);
        }
        let oracle = dir.path().join("cli-joined.mp4");
        join_cli_reference(&reference_cuts, &oracle).await;
        assert_eq!(packet_hashes(&output).await, packet_hashes(&oracle).await);
        let (actual_stream, actual_frames) = video_timeline(&output).await;
        let (expected_stream, expected_frames) = video_timeline(&oracle).await;
        assert_eq!(actual_stream, expected_stream, "joined video stream");
        assert_eq!(actual_frames.len(), expected_frames.len(), "joined frames");
        for (index, (actual, expected)) in actual_frames.iter().zip(&expected_frames).enumerate() {
            assert_eq!(actual, expected, "joined frame {index}");
        }
        let joined = ffmpeg_utils::extract_video_metadata(&output)
            .await
            .unwrap()
            .duration;
        let expected = ffmpeg_utils::extract_video_metadata(&oracle)
            .await
            .unwrap()
            .duration;
        assert_eq!(joined, expected, "joined metadata duration");
        for (covered, asked) in covered.iter().zip(&ranges) {
            assert!(covered.start <= asked.start + 0.05, "{covered:?} {asked:?}");
            assert_eq!(covered.end, asked.end);
        }
    }

    #[tokio::test]
    async fn whole_clip_without_settings_never_reencodes_mismatched_sources() {
        type Reporter = crate::progress::progress_reporter::ProgressReporter;
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let playlists = [
            create_test_playlist(first.path(), 0.5).await,
            create_test_playlist_with_size(second.path(), 0.5, "640x360").await,
        ];
        let mut sources = Vec::new();
        for playlist in &playlists {
            let video = playlist.with_extension("mp4");
            clip_from_playlist(None::<&Reporter>, playlist, &video, None)
                .await
                .unwrap();
            sources.push(video);
        }
        for count in [1, 2] {
            let output = first.path().join(format!("copy-{count}.mp4"));
            assert!(!concat_playlists_to_video(
                None::<&Reporter>,
                &playlists[..count]
                    .iter()
                    .map(|p| p.as_path())
                    .collect::<Vec<_>>(),
                vec![None; count],
                &crate::danmu2ass::Danmu2AssOptions::default(),
                &output,
                None,
            )
            .await
            .unwrap());
            let hashes = packet_hashes(&output).await;
            assert!(!hashes.is_empty());
            let oracle = first.path().join(format!("cli-copy-{count}.mp4"));
            join_cli_reference(&sources[..count], &oracle).await;
            assert_eq!(
                hashes,
                packet_hashes(&oracle).await,
                "copy mode must match CLI stream copy for video and audio packets"
            );
        }
    }

    #[tokio::test]
    async fn whole_clip_explicit_settings_cover_missing_empty_and_burned_danmu() {
        use crate::project::{EncodingPriority, RenderSettings, VideoCodec};
        use recorder::danmu::DanmuEntry;
        type Reporter = crate::progress::progress_reporter::ProgressReporter;

        let dir = tempfile::tempdir().unwrap();
        let playlist = create_test_playlist(dir.path(), 0.3).await;
        let options = crate::danmu2ass::Danmu2AssOptions::default();
        for (codec, name) in [(VideoCodec::Hevc, "hevc"), (VideoCodec::H264, "h264")] {
            for count in [1, 2] {
                for mode in ["missing", "empty", "burn"] {
                    let burn = mode == "burn";
                    let settings = RenderSettings {
                        codec,
                        priority: if burn {
                            EncodingPriority::Quality
                        } else {
                            EncodingPriority::Speed
                        },
                        bitrate_kbps: 4000,
                    };
                    let mut tracks = vec![None; count];
                    let entries = if burn {
                        vec![DanmuEntry {
                            ts: 0,
                            event_type: "danmu".into(),
                            content: "whole recording".into(),
                            emoji_content: None,
                            user_name: None,
                            price: None,
                            sc_duration: None,
                        }]
                    } else {
                        Vec::new()
                    };
                    if mode != "missing" {
                        tracks[0] = Some(entries);
                    }
                    let output = dir.path().join(format!("{name}-{count}-{mode}.mp4"));
                    let embedded = concat_playlists_to_video(
                        None::<&Reporter>,
                        &vec![playlist.as_path(); count],
                        tracks,
                        &options,
                        &output,
                        Some(&settings),
                    )
                    .await
                    .unwrap();
                    assert_eq!(embedded, burn);
                    let metadata = ffmpeg_utils::extract_video_metadata(&output).await.unwrap();
                    assert_eq!(metadata.video_codec, name);
                    if count == 1 {
                        // Even H.264 -> H.264 must encode rather than copy.
                        assert_eq!((metadata.width, metadata.height), (1920, 1080));
                    }
                    assert!(metadata.duration >= 0.25 * count as f64);
                    let decoded = ffmpeg_utils::ffmpeg_command()
                        .args(["-v", "error", "-i"])
                        .arg(&output)
                        .args(["-f", "null", "-"])
                        .output()
                        .await
                        .unwrap();
                    assert!(
                        decoded.status.success(),
                        "{}",
                        String::from_utf8_lossy(&decoded.stderr)
                    );
                }
            }
        }
        let legacy = dir.path().join("legacy.mp4");
        assert!(!concat_playlists_to_video(
            None::<&Reporter>,
            &[playlist.as_path()],
            vec![None],
            &options,
            &legacy,
            None,
        )
        .await
        .unwrap());
        let metadata = ffmpeg_utils::extract_video_metadata(&legacy).await.unwrap();
        assert_eq!(metadata.video_codec, "h264");
        assert_eq!((metadata.width, metadata.height), (320, 180));
    }

    #[tokio::test]
    async fn whole_clip_rejects_invalid_settings_before_creating_output() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("not-created/output.mp4");
        let invalid = crate::project::RenderSettings {
            bitrate_kbps: 0,
            ..crate::project::RenderSettings::default()
        };
        let error = concat_playlists_to_video(
            None::<&crate::progress::progress_reporter::ProgressReporter>,
            &[],
            Vec::new(),
            &crate::danmu2ass::Danmu2AssOptions::default(),
            &output,
            Some(&invalid),
        )
        .await
        .unwrap_err();
        assert_eq!(error, invalid.validate().unwrap_err());
        assert!(!output.parent().unwrap().exists());
    }
}
