//! Commands for the audio library: music, voiceover and other sound files
//! imported for the audio tracks of projects. They are kept in the output
//! directory, so every project and every client of this server shares them.

use std::path::{Path, PathBuf};

use chrono::{Local, Utc};
use ffmpeg_utils::ffmpeg_command;

use crate::database::audio::AudioRow;
use crate::ffmpeg::runner::TempArtifact;
use crate::handlers::project::{edit_project_timeline, ProjectBrief};
use crate::material::Peaks;
use crate::project::Timeline;
use crate::state::State;
use crate::state_type;

#[cfg(feature = "gui")]
use tauri::State as TauriState;

/// Directory of the library's files, relative to the output directory.
const AUDIO_DIR: &str = "audio";

async fn output_dir(state: &State) -> PathBuf {
    PathBuf::from(&state.config.read().await.output)
}

/// How a file with sound in `codec` is kept, as `(extension, codec options)`:
/// as it is where every web view plays and seeks it exactly, or else as AAC.
/// MP3 is encoded again too, as players only guess where to seek in one with a
/// variable bit rate, and the preview would not play what the export does.
fn kept_as(extension: &str, codec: &str) -> (&'static str, &'static [&'static str]) {
    const COPY: &[&str] = &["-c:a", "copy"];
    match (extension, codec) {
        ("wav", codec) if codec.starts_with("pcm_") => ("wav", COPY),
        ("flac", "flac") => ("flac", COPY),
        (_, "aac") => ("m4a", COPY),
        _ => ("m4a", &["-c:a", "aac", "-b:a", "256k"]),
    }
}

/// The name a file is listed by: `name`, or else that of the file.
fn display_name(name: Option<&str>, path: &Path) -> String {
    name.map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
}

/// Write the first sound of `source` to `target`, alone, with `codec` options.
async fn write_sound(source: &Path, target: &Path, codec: &[&str]) -> Result<(), String> {
    let mut command = ffmpeg_command();
    command
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"])
        .arg(source)
        .args(["-map", "0:a:0", "-vn", "-sn", "-dn"])
        .args(codec);
    if target
        .extension()
        .is_some_and(|extension| extension == "m4a")
    {
        // Its index first, for the preview to seek before it has loaded.
        command.args(["-movflags", "+faststart"]);
    }
    let output = command
        .arg(target)
        .output()
        .await
        .map_err(|e| format!("无法启动 FFmpeg: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "转换声音文件失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// The audio library, the latest import first.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_audio_files(state: state_type!()) -> Result<Vec<AudioRow>, String> {
    Ok(state.db.get_audio_files().await?)
}

/// Import the sound of the file at `file_path` into the audio library, listed
/// as `name` or else by the file's name.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn import_audio_file(
    state: state_type!(),
    file_path: String,
    name: Option<String>,
) -> Result<AudioRow, String> {
    let source = PathBuf::from(&file_path);
    if !source.is_file() {
        return Err("找不到要导入的文件".to_string());
    }
    let metadata = ffmpeg_utils::extract_video_metadata(&source).await?;
    if metadata.audio_codec.is_empty() {
        return Err("这个文件里没有声音".to_string());
    }
    let extension = source
        .extension()
        .map(|extension| extension.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let (extension, codec) = kept_as(&extension, &metadata.audio_codec);
    let file = format!(
        "{AUDIO_DIR}/{}-{}.{extension}",
        Utc::now().timestamp_millis(),
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let target = output_dir(&state).await.join(&file);
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let guard = TempArtifact::new(&target);
    write_sound(&source, &target, codec).await?;
    // Without the pictures some files carry, the container's length is the
    // sound's.
    let duration = ffmpeg_utils::extract_video_metadata(&target)
        .await?
        .duration;
    if duration <= 0.0 {
        return Err("无法读取声音的时长".to_string());
    }
    let size = tokio::fs::metadata(&target)
        .await
        .map_or(0, |metadata| metadata.len());
    let audio = state
        .db
        .add_audio_file(&AudioRow {
            id: 0,
            name: display_name(name.as_deref(), &source),
            file,
            duration,
            size: i64::try_from(size).unwrap_or(i64::MAX),
            created_at: Local::now().to_rfc3339(),
        })
        .await?;
    guard.keep();
    Ok(audio)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn rename_audio_file(state: state_type!(), id: i64, name: String) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("名称不能为空".to_string());
    }
    Ok(state.db.rename_audio_file(id, name).await?)
}

/// Projects whose audio tracks play the file.
async fn projects_using(state: &State, id: i64) -> Result<Vec<ProjectBrief>, String> {
    Ok(state
        .db
        .get_projects()
        .await?
        .into_iter()
        .filter(|project| Timeline::parse(&project.timeline).is_ok_and(|t| t.uses_audio(id)))
        .map(|project| ProjectBrief {
            id: project.id,
            title: project.title,
        })
        .collect())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_audio_usage(state: state_type!(), id: i64) -> Result<Vec<ProjectBrief>, String> {
    projects_using(&state, id).await
}

/// Delete a file of the library, with the audio items playing it.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn delete_audio_file(state: state_type!(), id: i64) -> Result<(), String> {
    let audio = state.db.get_audio_file(id).await?;
    for project in projects_using(&state, id).await? {
        edit_project_timeline(&state.db, project.id, |timeline| timeline.remove_audio(id)).await?;
    }
    state.db.delete_audio_file(id).await?;
    let path = output_dir(&state).await.join(&audio.file);
    for file in [path.clone(), crate::material::peaks_path(&path)] {
        if let Err(e) = tokio::fs::remove_file(&file).await {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("删除声音文件失败: {} - {e}", file.display());
            }
        }
    }
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_audio_peaks(state: state_type!(), id: i64) -> Result<Peaks, String> {
    let audio = state.db.get_audio_file(id).await?;
    crate::material::peaks(&output_dir(&state).await.join(&audio.file)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_that_play_and_seek_everywhere_are_kept_and_others_encoded() {
        assert_eq!(kept_as("wav", "pcm_s16le").0, "wav");
        assert_eq!(kept_as("flac", "flac").0, "flac");
        assert_eq!(kept_as("aac", "aac"), ("m4a", &["-c:a", "copy"][..]));
        assert_eq!(kept_as("m4a", "aac").0, "m4a");
        for (extension, codec) in [
            ("mp3", "mp3"),
            ("ogg", "vorbis"),
            ("m4a", "alac"),
            ("wav", "mp3"),
        ] {
            assert_eq!(
                kept_as(extension, codec),
                ("m4a", &["-c:a", "aac", "-b:a", "256k"][..]),
                "{extension} {codec}"
            );
        }
    }

    #[test]
    fn files_are_listed_by_the_name_given_or_their_own() {
        let path = Path::new("/music/片头曲.mp3");
        assert_eq!(display_name(Some(" BGM "), path), "BGM");
        assert_eq!(display_name(Some("  "), path), "片头曲");
        assert_eq!(display_name(None, path), "片头曲");
    }
}
