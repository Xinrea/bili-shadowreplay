//! Render a project (切片) timeline into one video.
//!
//! Every clip is a separate input seeked with `-ss/-t`, so FFmpeg only decodes
//! the parts that are used even when the material is hours long. Clips are
//! normalized to one canvas, frame rate and audio format, joined with `concat`,
//! and then overlays, subtitles and danmaku are drawn on the joined timeline,
//! in that order. Video overlays are inputs of their own like clips, and
//! their sound is mixed into the joined clips'.

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

/// Something laid over the joined clips from `start` to `end`, project time.
pub struct RenderOverlay {
    pub start: f64,
    pub end: f64,
    pub layer: OverlayLayer,
}

pub enum OverlayLayer {
    /// A PNG the editor drew, placed with its top left at `x`, `y`.
    Image {
        png: Vec<u8>,
        x: f64,
        y: f64,
    },
    Video(VideoLayer),
}

pub struct VideoLayer {
    pub path: PathBuf,
    /// In point within the file, in seconds.
    pub source_start: f64,
    pub has_audio: bool,
    /// Center on the 1920×1080 picture.
    pub x: f64,
    pub y: f64,
    /// The height follows the file's aspect ratio.
    pub width: f64,
    /// Degrees, clockwise.
    pub rotation: f64,
    /// 0 to 1.
    pub volume: f64,
}

pub struct ProjectRender<'a> {
    pub clips: Vec<RenderClip>,
    /// Drawn in order, so each covers those before it.
    pub overlays: Vec<RenderOverlay>,
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

/// Filter graph joining the clips into `[joined]` (video) and `[{audio}]`.
///
/// Each clip is padded and cut to exactly its timeline length, so a file a
/// little shorter than its clip cannot shift the subtitles and danmaku of the
/// clips after it.
fn concat_filter(clips: &[RenderClip], fps: f64, audio: &str) -> String {
    let mut filter = String::new();
    let mut inputs = String::new();
    for (index, clip) in clips.iter().enumerate() {
        let duration = clip.duration;
        filter.push_str(&format!(
            "[{index}:v]{},setsar=1,fps={fps},format=yuv420p,setpts=PTS-STARTPTS,\
             tpad=stop_mode=clone:stop_duration={duration:.6},trim=duration={duration:.6}[pv{index}];\n",
            hwaccel::H264_SCALE_PAD_FILTER,
        ));
        if clip.has_audio {
            filter.push_str(&format!(
                "[{index}:a]aresample=48000:async=1:first_pts=0,\
                 aformat=sample_fmts=fltp:channel_layouts=stereo,asetpts=PTS-STARTPTS,\
                 apad,atrim=duration={duration:.6}[pa{index}];\n"
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
        "{inputs}concat=n={}:v=1:a=1[joined][{audio}];\n",
        clips.len()
    ));
    filter
}

/// An overlay graph laid over `[{video}]`, with the inputs of `overlays`
/// starting at `first_input`.
struct OverlayGraph {
    filter: String,
    /// The video with every overlay on it.
    video: String,
    /// The sound of each video overlay, `total` long with silence around it.
    audio: Vec<String>,
}

fn overlay_filter(
    overlays: &[RenderOverlay],
    first_input: usize,
    fps: f64,
    total: f64,
    video: &str,
) -> OverlayGraph {
    let mut filter = String::new();
    let mut current = video.to_string();
    let mut audio = Vec::new();
    for (index, overlay) in overlays.iter().enumerate() {
        let input = first_input + index;
        let (start, end) = (overlay.start, overlay.end);
        let enable = format!("enable='gte(t,{start:.6})*lt(t,{end:.6})'");
        match &overlay.layer {
            OverlayLayer::Image { x, y, .. } => {
                filter.push_str(&format!(
                    "[{input}:v]format=rgba[ov{index}];\n\
                     [{current}][ov{index}]overlay=x={}:y={}:{enable}[layer{index}];\n",
                    x.round() as i64,
                    y.round() as i64,
                ));
            }
            OverlayLayer::Video(layer) => {
                // Even, as the encoder's chroma planes need.
                let width = ((layer.width / 2.0).round() * 2.0).max(2.0) as i64;
                let rotate = if layer.rotation.abs() > 0.01 {
                    let angle = layer.rotation.to_radians();
                    // Transparent corners: `none` would leave them unpainted.
                    format!(",rotate={angle:.6}:c=black@0:ow=rotw({angle:.6}):oh=roth({angle:.6})")
                } else {
                    String::new()
                };
                // Shifted to its place on the timeline; the joined clips show
                // through before it starts and after its file ends.
                filter.push_str(&format!(
                    "[{input}:v]setpts=PTS-STARTPTS,fps={fps},scale={width}:-2,setsar=1,\
                     format=yuva420p{rotate},setpts=PTS+{start:.6}/TB[ov{index}];\n\
                     [{current}][ov{index}]overlay=x={:.3}-overlay_w/2:y={:.3}-overlay_h/2:\
                     eof_action=pass:{enable}[layer{index}];\n",
                    layer.x, layer.y,
                ));
                if layer.has_audio && layer.volume > 0.0 {
                    let delay = (start * 1000.0).round() as i64;
                    filter.push_str(&format!(
                        "[{input}:a]aresample=48000:async=1:first_pts=0,\
                         aformat=sample_fmts=fltp:channel_layouts=stereo,asetpts=PTS-STARTPTS,\
                         atrim=duration={:.6},volume={:.3},adelay={delay}|{delay},\
                         apad,atrim=duration={total:.6}[oa{index}];\n",
                        end - start,
                        layer.volume.min(1.0),
                    ));
                    audio.push(format!("oa{index}"));
                }
            }
        }
        current = format!("layer{index}");
    }
    OverlayGraph {
        filter,
        video: current,
        audio,
    }
}

/// Mix `[main]` with the overlays' sound into `[outa]`. Every input lasts the
/// whole project, so amix scales each by as much and the volume filter
/// restores them: overlays add to the clips' sound instead of lowering it.
fn mix_filter(main: &str, overlays: &[String]) -> String {
    let inputs: String = std::iter::once(main)
        .chain(overlays.iter().map(String::as_str))
        .map(|label| format!("[{label}]"))
        .collect();
    let count = overlays.len() + 1;
    format!(
        "{inputs}amix=inputs={count}:duration=first:dropout_transition=0,volume={count}[outa];\n"
    )
}

/// The overlay picture's file in the render's directory, a name of our own.
fn overlay_image_name(index: usize) -> String {
    format!("overlay{index}.png")
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

    let overlays = overlay_filter(&job.overlays, job.clips.len(), fps, total, "joined");
    let joined_audio = if overlays.audio.is_empty() {
        "outa"
    } else {
        "maina"
    };
    let mut filter = concat_filter(&job.clips, fps, joined_audio);
    filter.push_str(&overlays.filter);
    if !overlays.audio.is_empty() {
        filter.push_str(&mix_filter(joined_audio, &overlays.audio));
    }
    for (index, overlay) in job.overlays.iter().enumerate() {
        if let OverlayLayer::Image { png, .. } = &overlay.layer {
            tokio::fs::write(directory.path().join(overlay_image_name(index)), png)
                .await
                .map_err(|error| error.to_string())?;
        }
    }
    let mut current = overlays.video;
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
            job.clips.len() + job.overlays.len(),
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
    for (index, overlay) in job.overlays.iter().enumerate() {
        match &overlay.layer {
            OverlayLayer::Image { .. } => {
                command
                    .args(["-threads", "1", "-i"])
                    .arg(overlay_image_name(index));
            }
            OverlayLayer::Video(layer) => {
                let input = std::path::absolute(&layer.path).map_err(|error| error.to_string())?;
                command
                    .args(["-ss", &format!("{:.6}", layer.source_start)])
                    .args(["-t", &format!("{:.6}", overlay.end - overlay.start)])
                    .arg("-i")
                    .arg(input);
            }
        }
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
        let filter = concat_filter(&[clip(true, 30.0), clip(false, 60.0)], 60.0, "outa");
        assert!(filter.contains("[0:v]scale=1920:1080"));
        // Both streams of a clip last exactly its timeline duration.
        assert!(filter.contains(
            "fps=60,format=yuv420p,setpts=PTS-STARTPTS,\
             tpad=stop_mode=clone:stop_duration=2.500000,trim=duration=2.500000[pv0]"
        ));
        assert!(filter.contains("[0:a]aresample=48000"));
        assert!(filter.contains("apad,atrim=duration=2.500000[pa0]"));
        // A clip without audio gets silence of the same length.
        assert!(filter.contains("anullsrc=r=48000:cl=stereo,atrim=duration=2.500000"));
        assert!(!filter.contains("[1:a]"));
        assert!(filter.contains("[pv0][pa0][pv1][pa1]concat=n=2:v=1:a=1[joined][outa]"));
    }

    fn video_layer(has_audio: bool, rotation: f64) -> OverlayLayer {
        OverlayLayer::Video(VideoLayer {
            path: PathBuf::from("pip.mp4"),
            source_start: 3.0,
            has_audio,
            x: 1500.0,
            y: 800.0,
            width: 641.0,
            rotation,
            volume: 0.5,
        })
    }

    #[test]
    fn overlays_are_laid_over_the_joined_clips_in_order() {
        let overlays = [
            RenderOverlay {
                start: 1.0,
                end: 4.0,
                layer: video_layer(true, 0.0),
            },
            RenderOverlay {
                start: 2.0,
                end: 3.5,
                layer: OverlayLayer::Image {
                    png: Vec::new(),
                    x: -10.4,
                    y: 20.6,
                },
            },
            RenderOverlay {
                start: 0.0,
                end: 9.0,
                layer: video_layer(false, 90.0),
            },
        ];
        let graph = overlay_filter(&overlays, 2, 30.0, 10.0, "joined");
        let filter = &graph.filter;
        // The video is scaled to an even width and moved to its place in time.
        assert!(filter.contains(
            "[2:v]setpts=PTS-STARTPTS,fps=30,scale=642:-2,setsar=1,format=yuva420p,setpts=PTS+1.000000/TB[ov0]"
        ));
        assert!(filter.contains(
            "[joined][ov0]overlay=x=1500.000-overlay_w/2:y=800.000-overlay_h/2:eof_action=pass:\
             enable='gte(t,1.000000)*lt(t,4.000000)'[layer0]"
        ));
        // Its sound starts with it and lasts the whole project.
        assert!(filter.contains("atrim=duration=3.000000,volume=0.500,adelay=1000|1000,apad,atrim=duration=10.000000[oa0]"));
        // Pictures are placed by their top left, each over the layer before.
        assert!(filter.contains("[3:v]format=rgba[ov1]"));
        assert!(filter.contains(
            "[layer0][ov1]overlay=x=-10:y=21:enable='gte(t,2.000000)*lt(t,3.500000)'[layer1]"
        ));
        assert!(filter.contains("rotate=1.570796:c=black@0:ow=rotw(1.570796):oh=roth(1.570796)"));
        assert!(!filter.contains("[4:a]"));
        assert_eq!(
            (graph.video.as_str(), graph.audio.as_slice()),
            ("layer2", &["oa0".to_string()][..])
        );
        assert_eq!(
            mix_filter("maina", &graph.audio),
            "[maina][oa0]amix=inputs=2:duration=first:dropout_transition=0,volume=2[outa];\n"
        );
        // Without overlays the joined clips are used as they are.
        let none = overlay_filter(&[], 2, 30.0, 10.0, "joined");
        assert_eq!((none.filter.as_str(), none.video.as_str()), ("", "joined"));
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

    #[tokio::test]
    #[ignore = "requires FFmpeg with libx264"]
    async fn a_clip_longer_than_its_file_keeps_its_timeline_length() {
        let directory = tempfile::tempdir().unwrap();
        let red = directory.path().join("red.mp4");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=320x180:r=30:d=2",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2",
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
                "color=c=blue:s=320x180:r=30:d=2",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
            ],
            &blue,
        )
        .await;
        let output = directory.path().join("out.mp4");
        let clip = |path: &Path, duration: f64, has_audio: bool| RenderClip {
            path: path.to_path_buf(),
            start: 0.0,
            duration,
            has_audio,
            fps: 30.0,
        };
        // The timeline allows a clip to end a little after its file.
        render_project(
            &NoopReporter,
            ProjectRender {
                clips: vec![clip(&red, 2.4, true), clip(&blue, 2.0, false)],
                overlays: Vec::new(),
                subtitle_ass: None,
                fonts_directory: None,
                danmus: Vec::new(),
                danmu_options: Danmu2AssOptions::default(),
                output: &output,
            },
        )
        .await
        .unwrap();
        let metadata = ffmpeg_utils::extract_video_metadata(&output).await.unwrap();
        assert!(
            (metadata.duration - 4.4).abs() < 0.1,
            "{}",
            metadata.duration
        );
        // The second clip starts where the timeline puts it, not early.
        let padded = average_color(&output, 2.2).await;
        assert!(padded[0] > 150 && padded[2] < 80, "{padded:?}");
        let second = average_color(&output, 2.6).await;
        assert!(second[2] > 150 && second[0] < 80, "{second:?}");
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
                overlays: Vec::new(),
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

    /// Average RGB of the `width`×`height` area at `x`, `y` in the frame at `seconds`.
    async fn area_color(file: &Path, seconds: f64, [x, y, width, height]: [u32; 4]) -> [u8; 3] {
        let result = ffmpeg_command()
            .args(["-v", "error", "-ss", &seconds.to_string(), "-i"])
            .arg(file)
            .args(["-frames:v", "1", "-vf"])
            .arg(format!("crop={width}:{height}:{x}:{y},scale=1:1"))
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .output()
            .await
            .unwrap();
        assert!(result.status.success());
        [result.stdout[0], result.stdout[1], result.stdout[2]]
    }

    /// Mean volume in dB of `duration` seconds of sound from `seconds` on.
    async fn mean_volume(file: &Path, seconds: f64, duration: f64) -> f64 {
        let result = ffmpeg_command()
            .args([
                "-hide_banner",
                "-nostdin",
                "-ss",
                &seconds.to_string(),
                "-t",
            ])
            .arg(duration.to_string())
            .arg("-i")
            .arg(file)
            .args(["-af", "volumedetect", "-f", "null", "-"])
            .output()
            .await
            .unwrap();
        let log = String::from_utf8_lossy(&result.stderr);
        log.lines()
            .find_map(|line| line.split("mean_volume: ").nth(1))
            .and_then(|value| value.trim_end_matches(" dB").trim().parse().ok())
            .unwrap_or_else(|| panic!("no volume in {log}"))
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg with libx264"]
    async fn ffmpeg_lays_videos_and_pictures_over_the_clips() {
        let directory = tempfile::tempdir().unwrap();
        // A silent red clip under a blue video with a tone and a green picture.
        let red = directory.path().join("red.mp4");
        run_ffmpeg(
            &["-f", "lavfi", "-i", "color=c=red:s=320x180:r=30:d=4"]
                .into_iter()
                .chain(["-c:v", "libx264", "-preset", "ultrafast"])
                .collect::<Vec<_>>(),
            &red,
        )
        .await;
        let blue = directory.path().join("blue.mp4");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=320x180:r=30:d=6",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=6",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-c:a",
                "aac",
            ],
            &blue,
        )
        .await;
        let green = directory.path().join("green.png");
        run_ffmpeg(
            &[
                "-f",
                "lavfi",
                "-i",
                "color=c=lime:s=200x100",
                "-frames:v",
                "1",
            ],
            &green,
        )
        .await;
        let video =
            |start: f64, end: f64, x: f64, y: f64, width: f64, rotation: f64| RenderOverlay {
                start,
                end,
                layer: OverlayLayer::Video(VideoLayer {
                    path: blue.clone(),
                    source_start: 2.0,
                    has_audio: true,
                    x,
                    y,
                    width,
                    rotation,
                    volume: 1.0,
                }),
            };
        let output = directory.path().join("out.mp4");
        render_project(
            &NoopReporter,
            ProjectRender {
                clips: vec![RenderClip {
                    path: red.clone(),
                    start: 0.0,
                    duration: 4.0,
                    has_audio: false,
                    fps: 30.0,
                }],
                overlays: vec![
                    // The top left quarter from 1 s to 3 s.
                    video(1.0, 3.0, 480.0, 270.0, 960.0, 0.0),
                    RenderOverlay {
                        start: 2.0,
                        end: 3.5,
                        layer: OverlayLayer::Image {
                            png: std::fs::read(&green).unwrap(),
                            x: 1600.0,
                            y: 900.0,
                        },
                    },
                    // Turned on its side, still centered where it was placed.
                    video(0.0, 1.0, 1440.0, 540.0, 400.0, 90.0),
                ],
                subtitle_ass: None,
                fonts_directory: None,
                danmus: Vec::new(),
                danmu_options: Danmu2AssOptions::default(),
                output: &output,
            },
        )
        .await
        .unwrap();

        let quarter = [100, 100, 760, 340];
        let is = |color: [u8; 3], [r, g, b]: [bool; 3]| {
            let high = |value: u8, on: bool| if on { value > 150 } else { value < 80 };
            high(color[0], r) && high(color[1], g) && high(color[2], b)
        };
        let (red, blue, green) = (
            [true, false, false],
            [false, false, true],
            [false, true, false],
        );
        for (seconds, area, expected) in [
            (0.5, quarter, red),
            (1.5, quarter, blue),
            (3.5, quarter, red),
            (1.5, [1620, 910, 160, 80], red),
            (2.5, [1620, 910, 160, 80], green),
            (3.8, [1620, 910, 160, 80], red),
            // 400 wide and 225 high on its side: blue 1440±100 across, red past it.
            (0.5, [1360, 330, 160, 420], blue),
            (0.5, [1560, 330, 60, 420], red),
            (1.5, [1360, 330, 160, 420], red),
        ] {
            let color = area_color(&output, seconds, area).await;
            assert!(is(color, expected), "{seconds} s {area:?}: {color:?}");
        }
        // The video's tone plays while it is shown, over the silent clip.
        assert!(mean_volume(&output, 1.2, 0.6).await > -40.0);
        assert!(mean_volume(&output, 3.2, 0.6).await < -60.0);
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
