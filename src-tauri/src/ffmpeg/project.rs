//! Render a project (切片) timeline into one video.
//!
//! Every clip is a separate input seeked with `-ss/-t`, so FFmpeg only decodes
//! the parts that are used even when the material is hours long. Clips are
//! normalized to one canvas, frame rate and audio format, joined with `concat`,
//! and then subtitles and danmaku are drawn on the joined timeline.

use std::path::{Path, PathBuf};

use ffmpeg_utils::ffmpeg_command;
use recorder::danmu::DanmuEntry;

use crate::danmu2ass::Danmu2AssOptions;
use crate::progress::progress_reporter::ProgressReporterTrait;

use super::{
    danmu, hwaccel,
    runner::{run_command, ProgressMode, ProgressTotal, TempArtifact},
};

/// Output frame rate when no clip reports one.
const DEFAULT_FPS: f64 = 30.0;
const MAX_FPS: f64 = 60.0;

pub struct RenderClip {
    pub path: PathBuf,
    /// In point within the file, in seconds.
    pub start: f64,
    pub duration: f64,
    pub has_audio: bool,
    /// Frame rate of the file, `0.0` when unknown.
    pub fps: f64,
}

pub struct ProjectRender<'a> {
    pub clips: Vec<RenderClip>,
    /// ASS document in project time, exactly as previewed by the editor.
    pub subtitle_ass: Option<String>,
    /// Font files the preview used for the subtitles.
    pub fonts_directory: Option<&'a Path>,
    /// Danmaku in project time; empty to burn none.
    pub danmus: Vec<DanmuEntry>,
    pub danmu_options: Danmu2AssOptions,
    pub output: &'a Path,
}

fn output_fps(clips: &[RenderClip], danmu: bool) -> f64 {
    let fps = clips
        .iter()
        .map(|clip| clip.fps)
        .filter(|fps| fps.is_finite() && *fps > 0.0)
        .fold(0.0, f64::max);
    let fps = if fps > 0.0 {
        fps.min(MAX_FPS)
    } else {
        DEFAULT_FPS
    };
    if danmu {
        danmu::danmu_output_fps(fps)
    } else {
        // Keep the usual rational rates (29.97, 59.94) exact enough for ffmpeg.
        (fps * 1000.0).round() / 1000.0
    }
}

/// Filter graph joining the clips into `[joined]` (video) and `[outa]` (audio).
fn concat_filter(clips: &[RenderClip], fps: f64) -> String {
    let mut filter = String::new();
    let mut inputs = String::new();
    for (index, clip) in clips.iter().enumerate() {
        filter.push_str(&format!(
            "[{index}:v]{},setsar=1,fps={fps},format=yuv420p,setpts=PTS-STARTPTS[pv{index}];\n",
            hwaccel::H264_SCALE_PAD_FILTER,
        ));
        if clip.has_audio {
            filter.push_str(&format!(
                "[{index}:a]aresample=48000:async=1:first_pts=0,\
                 aformat=sample_fmts=fltp:channel_layouts=stereo,asetpts=PTS-STARTPTS[pa{index}];\n"
            ));
        } else {
            filter.push_str(&format!(
                "anullsrc=r=48000:cl=stereo,atrim=duration={:.6},\
                 aformat=sample_fmts=fltp:channel_layouts=stereo[pa{index}];\n",
                clip.duration
            ));
        }
        inputs.push_str(&format!("[pv{index}][pa{index}]"));
    }
    filter.push_str(&format!(
        "{inputs}concat=n={}:v=1:a=1[joined][outa];\n",
        clips.len()
    ));
    filter
}

pub async fn render_project<R: ProgressReporterTrait>(
    reporter: &R,
    job: ProjectRender<'_>,
) -> Result<(), String> {
    if job.clips.is_empty() {
        return Err("时间轴上没有片段".to_string());
    }
    // Generated files use fixed names in a private directory that is FFmpeg's
    // working directory, so no user path is interpolated into the graph.
    let directory = tempfile::Builder::new()
        .prefix("bili-project-")
        .tempdir()
        .map_err(|error| error.to_string())?;
    let burn_danmu = !job.danmus.is_empty();
    let fps = output_fps(&job.clips, burn_danmu);
    let total: f64 = job.clips.iter().map(|clip| clip.duration).sum();

    let mut filter = concat_filter(&job.clips, fps);
    let mut current = "joined".to_string();
    if let Some(ass) = &job.subtitle_ass {
        tokio::fs::write(directory.path().join("subtitle.ass"), ass)
            .await
            .map_err(|error| error.to_string())?;
        filter.push_str(&format!("[{current}]ass=filename=subtitle.ass"));
        if let Some(fonts) = job.fonts_directory {
            filter.push_str(&format!(
                ":fontsdir={}",
                super::subtitle::subtitle_filter_path(fonts)?
            ));
        }
        filter.push_str("[subtitled];\n");
        current = "subtitled".to_string();
    }
    let mut image_inputs = Vec::new();
    if burn_danmu {
        let images = danmu::prepare_danmu_layers(
            Some(reporter),
            &job.danmus,
            &job.danmu_options,
            directory.path(),
        )
        .await?;
        filter.push_str(&format!("[{current}]ass=danmu.ass[danmu0];\n"));
        let (chain, slots) = danmu::image_overlay_chain(
            &images,
            job.danmu_options.opacity,
            job.clips.len(),
            "danmu",
        );
        filter.push_str(&chain.filter);
        image_inputs = chain.inputs;
        current = format!("danmu{slots}");
    }
    let encoder = hwaccel::get_x264_encoder().await;
    let format = if hwaccel::is_vaapi_encoder(encoder) {
        hwaccel::vaapi_filter_suffix()
    } else {
        "format=yuv420p"
    };
    filter.push_str(&format!("[{current}]{format}[outv]\n"));
    tokio::fs::write(directory.path().join("filter.txt"), &filter)
        .await
        .map_err(|error| error.to_string())?;

    let output = std::path::absolute(job.output).map_err(|error| error.to_string())?;
    let output_guard = TempArtifact::new(&output);
    let mut command = ffmpeg_command();
    command.current_dir(directory.path()).arg("-nostdin");
    for clip in &job.clips {
        let input = std::path::absolute(&clip.path).map_err(|error| error.to_string())?;
        command
            .args(["-ss", &format!("{:.6}", clip.start)])
            .args(["-t", &format!("{:.6}", clip.duration)])
            .arg("-i")
            .arg(input);
    }
    for image in image_inputs {
        // A single PNG frame is repeated by overlay framesync.
        command.args(["-threads", "1", "-i"]).arg(image);
    }
    command
        .arg(danmu::filter_complex_file_option().await)
        .arg("filter.txt")
        .args(["-map", "[outv]", "-map", "[outa]"]);
    hwaccel::apply_x264_encoder_only(&mut command, encoder);
    hwaccel::apply_x264_quality_args(&mut command, encoder);
    command
        .args(["-c:a", "aac", "-b:a", "192k", "-r", &fps.to_string()])
        .args(["-movflags", "+faststart", "-y"])
        .arg(&output);
    log::info!("Render project: {command:?}");
    log::debug!("Project filter graph:\n{filter}");
    run_command(
        command,
        ProgressMode::stage("导出切片", ProgressTotal::Known(total)),
        "Render project",
        Some(reporter),
    )
    .await
    .map_err(|error| error.to_string())?;
    output_guard.keep();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(has_audio: bool, fps: f64) -> RenderClip {
        RenderClip {
            path: PathBuf::from("a.mp4"),
            start: 1.0,
            duration: 2.5,
            has_audio,
            fps,
        }
    }

    #[test]
    fn clips_are_normalized_and_joined_in_order() {
        let filter = concat_filter(&[clip(true, 30.0), clip(false, 60.0)], 60.0);
        assert!(filter.contains("[0:v]scale=1920:1080"));
        assert!(filter.contains("fps=60,format=yuv420p,setpts=PTS-STARTPTS[pv0]"));
        assert!(filter.contains("[0:a]aresample=48000"));
        // A clip without audio gets silence of the same length.
        assert!(filter.contains("anullsrc=r=48000:cl=stereo,atrim=duration=2.500000"));
        assert!(!filter.contains("[1:a]"));
        assert!(filter.contains("[pv0][pa0][pv1][pa1]concat=n=2:v=1:a=1[joined][outa]"));
    }

    #[test]
    fn frame_rate_follows_the_fastest_clip_within_limits() {
        assert_eq!(
            output_fps(&[clip(true, 25.0), clip(true, 29.97)], false),
            29.97
        );
        assert_eq!(output_fps(&[clip(true, 120.0)], false), 60.0);
        assert_eq!(output_fps(&[clip(true, 0.0)], false), DEFAULT_FPS);
        // Danmaku scrolls smoothly only at 60 FPS or more.
        assert_eq!(output_fps(&[clip(true, 30.0)], true), 60.0);
    }

    #[derive(Clone)]
    struct NoopReporter;

    #[async_trait::async_trait]
    impl ProgressReporterTrait for NoopReporter {
        async fn update(&self, _content: &str) {}
        async fn finish(&self, _success: bool, _message: &str) {}
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

    /// Average RGB of the frame at `seconds`, scaled down to one pixel.
    async fn average_color(file: &Path, seconds: f64) -> [u8; 3] {
        let result = ffmpeg_command()
            .args(["-v", "error", "-ss", &seconds.to_string(), "-i"])
            .arg(file)
            .args([
                "-frames:v",
                "1",
                "-vf",
                "crop=iw/2:ih/2,scale=1:1",
                "-f",
                "rawvideo",
            ])
            .args(["-pix_fmt", "rgb24", "-"])
            .output()
            .await
            .unwrap();
        assert!(result.status.success());
        [result.stdout[0], result.stdout[1], result.stdout[2]]
    }

    fn text_danmu(ts: i64) -> DanmuEntry {
        DanmuEntry {
            ts,
            event_type: "danmu".into(),
            content: "弹幕".into(),
            emoji_content: None,
            user_name: None,
            price: None,
            sc_duration: None,
        }
    }

    /// Render 1-3 s of a red 30 FPS clip and 0.5-2.5 s of a larger blue
    /// 60 FPS clip without audio, then check the joined result.
    async fn render_two_clips(subtitles: bool, danmaku: bool) {
        let directory = tempfile::Builder::new()
            .prefix("project [it's]")
            .tempdir()
            .unwrap();
        let red = directory.path().join("red clip.mp4");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=320x180:r=30:d=4",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=4",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-c:a",
                "aac",
            ],
            &red,
        )
        .await;
        let blue = directory.path().join("blue.mp4");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=640x360:r=60:d=3",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
            ],
            &blue,
        )
        .await;
        let output = directory.path().join("out.mp4");
        let ass = "[Script Info]\nScriptType: v4.00+\nPlayResX: 1920\nPlayResY: 1080\n\n\
            [V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour\n\
            Style: Default,Arial,48,&H00FFFFFF\n\n[Events]\n\
            Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
            Dialogue: 0,0:00:00.50,0:00:03.50,Default,,0,0,0,,hello\n";
        render_project(
            &NoopReporter,
            ProjectRender {
                clips: vec![
                    RenderClip {
                        path: red.clone(),
                        start: 1.0,
                        duration: 2.0,
                        has_audio: true,
                        fps: 30.0,
                    },
                    RenderClip {
                        path: blue.clone(),
                        start: 0.5,
                        duration: 2.0,
                        has_audio: false,
                        fps: 60.0,
                    },
                ],
                subtitle_ass: subtitles.then(|| ass.to_string()),
                fonts_directory: None,
                danmus: if danmaku {
                    vec![text_danmu(500), text_danmu(2500)]
                } else {
                    Vec::new()
                },
                danmu_options: Danmu2AssOptions::default(),
                output: &output,
            },
        )
        .await
        .unwrap();
        let metadata = ffmpeg_utils::extract_video_metadata(&output).await.unwrap();
        assert_eq!((metadata.width, metadata.height), (1920, 1080));
        assert_eq!(metadata.fps, 60.0);
        assert!(!metadata.audio_codec.is_empty());
        assert!(
            (metadata.duration - 4.0).abs() < 0.1,
            "{}",
            metadata.duration
        );
        let first = average_color(&output, 1.0).await;
        assert!(first[0] > 150 && first[2] < 80, "{first:?}");
        let second = average_color(&output, 3.0).await;
        assert!(second[2] > 150 && second[0] < 80, "{second:?}");
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg with libx264"]
    async fn ffmpeg_renders_trimmed_clips_on_one_canvas() {
        render_two_clips(false, false).await;
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg with libass and libx264"]
    async fn ffmpeg_renders_subtitles_and_danmaku_on_the_joined_timeline() {
        render_two_clips(true, true).await;
    }
}
