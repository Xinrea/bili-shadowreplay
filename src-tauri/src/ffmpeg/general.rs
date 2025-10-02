use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use async_ffmpeg_sidecar::{event::FfmpegEvent, log_parser::FfmpegLogParser};
use tokio::io::{AsyncWriteExt, BufReader};

use crate::progress::progress_reporter::ProgressReporterTrait;

use super::ffmpeg_path;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;
#[cfg(target_os = "windows")]
#[allow(unused_imports)]
use std::os::windows::process::CommandExt;

/// Generate a random filename in hex
pub async fn random_filename() -> String {
    format!("{:x}", rand::random::<u64>())
}

pub async fn handle_ffmpeg_process(
    reporter: Option<&impl ProgressReporterTrait>,
    ffmpeg_process: &mut tokio::process::Command,
) -> Result<(), String> {
    let child = ffmpeg_process
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn();
    if let Err(e) = child {
        return Err(e.to_string());
    }
    let mut child = child.unwrap();
    let stderr = child.stderr.take().unwrap();
    let reader = BufReader::new(stderr);
    let mut parser = FfmpegLogParser::new(reader);
    while let Ok(event) = parser.parse_next_event().await {
        match event {
            FfmpegEvent::Log(_level, content) => {
                // if contains "out_time_ms=66654667", by the way, it's actually in us
                if content.starts_with("out_time_ms") {
                    let time_str = content.strip_prefix("out_time_ms=").unwrap_or_default();
                    if let Some(reporter) = reporter {
                        reporter.update(time_str);
                    }
                }
            }
            FfmpegEvent::LogEOF => break,
            FfmpegEvent::Error(e) => {
                log::error!("[FFmpeg Error] {}", e.to_string());
                return Err(e.to_string());
            }
            _ => {}
        }
    }
    if let Err(e) = child.wait().await {
        return Err(e.to_string());
    }

    Ok(())
}

pub async fn concat_videos(
    reporter: Option<&impl ProgressReporterTrait>,
    videos: &[PathBuf],
    output_path: &Path,
) -> Result<(), String> {
    // ffmpeg -i input1.mp4 -i input2.mp4 -i input3.mp4 -c copy output.mp4
    let mut ffmpeg_process = tokio::process::Command::new(ffmpeg_path());
    #[cfg(target_os = "windows")]
    ffmpeg_process.creation_flags(CREATE_NO_WINDOW);

    let output_folder = output_path.parent().unwrap();
    if !output_folder.exists() {
        std::fs::create_dir_all(output_folder).unwrap();
    }

    let filelist_filename = format!("filelist_{}.txt", random_filename().await);

    let mut filelist = tokio::fs::File::create(&output_folder.join(&filelist_filename))
        .await
        .unwrap();
    for video in videos {
        filelist
            .write_all(format!("file '{}'\n", video.display()).as_bytes())
            .await
            .unwrap();
    }
    filelist.flush().await.unwrap();

    // Convert &[PathBuf] to &[&Path] for check_videos
    let video_refs: Vec<&Path> = videos.iter().map(|p| p.as_path()).collect();
    let should_encode = !super::check_videos(&video_refs).await;

    ffmpeg_process.args([
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
        output_folder.join(&filelist_filename).to_str().unwrap(),
    ]);
    if should_encode {
        ffmpeg_process.args(["-vf", "scale=1920:1080"]);
        ffmpeg_process.args(["-r", "60"]);
        ffmpeg_process.args(["-c", "libx264"]);
        ffmpeg_process.args(["-c:a", "aac"]);
        ffmpeg_process.args(["-b:v", "6000k"]);
        ffmpeg_process.args(["-b:a", "128k"]);
        ffmpeg_process.args(["-threads", "0"]);
    } else {
        ffmpeg_process.args(["-c", "copy"]);
    }
    ffmpeg_process.args([output_path.to_str().unwrap()]);
    ffmpeg_process.args(["-progress", "pipe:2"]);
    ffmpeg_process.args(["-y"]);

    handle_ffmpeg_process(reporter, &mut ffmpeg_process).await?;

    // clean up filelist
    let _ = tokio::fs::remove_file(output_folder.join(&filelist_filename)).await;

    Ok(())
}
