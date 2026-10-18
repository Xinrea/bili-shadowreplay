//! Commands for uploads (投稿): Bilibili submissions of a material, a project
//! or any video file.

use std::path::{Path, PathBuf};

use chrono::{Local, Utc};
use recorder::platforms::bilibili::profile::Profile;
use serde::Serialize;
use serde_json::json;
use tokio::sync::oneshot;

use crate::database::task::TaskRow;
use crate::database::upload::UploadRow;
use crate::events::AppEvent;
use crate::handlers::project::{
    material_danmu_video, material_title, project_cover_file, project_upload_video,
    remove_upload_renders, set_project_danmu, single_project_view, write_cover,
};
use crate::handlers::video::{get_optimal_thumbnail_timestamp, upload_video_file, PostedVideo};
use crate::progress::progress_reporter::{ProgressReporter, ProgressReporterTrait};
use crate::project::Timeline;
use crate::state::State;
use crate::state_type;

#[cfg(feature = "gui")]
use tauri::State as TauriState;

/// A file the static server serves under `base` (`output` or `cache`).
#[derive(Debug, Clone, Serialize)]
pub struct StaticFile {
    pub base: &'static str,
    pub path: String,
}

/// An upload with what the uploads page shows about its source.
#[derive(Debug, Clone, Serialize)]
pub struct UploadView {
    #[serde(flatten)]
    pub upload: UploadRow,
    /// Name of the material, project or file; empty when it no longer exists.
    pub source_title: String,
    /// `material` (uploaded as is, also a project of one untouched material),
    /// `render` (a project, or a material burning its danmaku, whose video is
    /// rendered when posting), `file`, or `missing`.
    pub source_state: &'static str,
    /// Burn danmaku into the posted video: the material's own setting, or the
    /// project's, which its editor shares.
    pub danmu: bool,
    /// The source has danmaku that could be burned.
    pub danmu_available: bool,
    /// Cover to show, relative to the output directory.
    pub display_cover: String,
    /// Video to pick cover frames from, when the static server can serve it.
    pub video: Option<StaticFile>,
}

async fn output_dir(state: &State) -> PathBuf {
    PathBuf::from(&state.config.read().await.output)
}

/// Directory of an upload's own files, relative to the output directory.
fn upload_dir(id: i64) -> String {
    format!("uploads/{id}")
}

/// `path` as served by the static server, if it is inside one of its folders.
async fn static_file(state: &State, path: &Path) -> Option<StaticFile> {
    let roots = {
        let config = state.config.read().await;
        [
            ("output", config.output.clone()),
            ("cache", config.cache.clone()),
        ]
    };
    roots.into_iter().find_map(|(base, root)| {
        let relative = path.strip_prefix(root).ok()?;
        Some(StaticFile {
            base,
            path: relative.to_string_lossy().replace('\\', "/"),
        })
    })
}

async fn upload_view(state: &State, upload: UploadRow) -> UploadView {
    let mut view = UploadView {
        source_title: String::new(),
        source_state: "missing",
        danmu: false,
        danmu_available: false,
        display_cover: upload.cover.clone(),
        video: None,
        upload,
    };
    match view.upload.source.as_str() {
        "material" => {
            if let Ok(material) = state.db.get_video(view.upload.source_id).await {
                view.source_title = material_title(&material);
                let file = output_dir(state).await.join(&material.file);
                view.danmu_available = crate::material::danmu_path(&file).is_file();
                view.danmu = view.upload.danmu;
                // Posting falls back to the file itself without saved danmaku.
                view.source_state = if view.danmu && view.danmu_available {
                    "render"
                } else {
                    "material"
                };
                if view.display_cover.is_empty() {
                    view.display_cover = material.cover.clone();
                }
                view.video = Some(StaticFile {
                    base: "output",
                    path: material.file,
                });
            }
        }
        "project" => {
            let Ok(project) = state.db.get_project(view.upload.source_id).await else {
                return view;
            };
            let Ok(project) = single_project_view(state, project).await else {
                return view;
            };
            view.source_title = project.project.title.clone();
            view.source_state = if project.needs_render {
                "render"
            } else {
                "material"
            };
            if view.display_cover.is_empty() {
                view.display_cover = project.display_cover.clone();
            }
            let timeline = Timeline::parse(&project.project.timeline).unwrap_or_default();
            view.danmu = timeline.danmu;
            let output = output_dir(state).await;
            for id in timeline.material_ids() {
                if let Ok(material) = state.db.get_video(id).await {
                    if crate::material::danmu_path(&output.join(&material.file)).is_file() {
                        view.danmu_available = true;
                        break;
                    }
                }
            }
            // Cover frames come from the first clip's material.
            if let Some(clip) = timeline.clips.first() {
                view.video = state
                    .db
                    .get_video(clip.material_id)
                    .await
                    .ok()
                    .map(|material| StaticFile {
                        base: "output",
                        path: material.file,
                    });
            }
        }
        _ => {
            let path = PathBuf::from(&view.upload.file);
            view.source_title = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            if path.is_file() {
                view.source_state = "file";
                view.video = static_file(state, &path).await;
            }
        }
    }
    view
}

/// Capture a cover for a video file, which has no thumbnail of its own.
async fn capture_file_cover(state: &State, upload: &UploadRow) -> Result<String, String> {
    let file = Path::new(&upload.file);
    let duration = ffmpeg_utils::extract_video_metadata(file).await?.duration;
    let relative = format!("{}/cover.jpg", upload_dir(upload.id));
    let target = output_dir(state).await.join(&relative);
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    crate::ffmpeg::extract_frame(file, get_optimal_thumbnail_timestamp(duration), &target).await?;
    Ok(relative)
}

fn ensure_editable(upload: &UploadRow) -> Result<(), String> {
    if upload.is_active() {
        Err("投稿进行中，暂时无法修改".to_string())
    } else if upload.status == "posted" {
        Err("已投稿的记录无法修改".to_string())
    } else {
        Ok(())
    }
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_uploads(state: state_type!()) -> Result<Vec<UploadView>, String> {
    let uploads = state.db.get_uploads().await?;
    let mut views = Vec::with_capacity(uploads.len());
    for upload in uploads {
        views.push(upload_view(&state, upload).await);
    }
    Ok(views)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_upload(state: state_type!(), id: i64) -> Result<UploadView, String> {
    let upload = state.db.get_upload(id).await?;
    Ok(upload_view(&state, upload).await)
}

/// Start a draft for `source`: a material or project `source_id`, or the
/// video `file`.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn create_upload(
    state: state_type!(),
    source: String,
    source_id: i64,
    file: String,
    profile: serde_json::Value,
) -> Result<UploadView, String> {
    let (source_id, file, room_id) = match source.as_str() {
        "material" => {
            let material = state
                .db
                .get_video(source_id)
                .await
                .map_err(|_| "素材不存在".to_string())?;
            (source_id, String::new(), material.room_id)
        }
        "project" => {
            let project = state
                .db
                .get_project(source_id)
                .await
                .map_err(|_| "切片不存在".to_string())?;
            (source_id, String::new(), project.room_id)
        }
        "file" => {
            if !Path::new(&file).is_file() {
                return Err("视频文件不存在".to_string());
            }
            (0, file, String::new())
        }
        _ => return Err(format!("未知的投稿来源: {source}")),
    };
    let now = Local::now().to_rfc3339();
    let mut upload = state
        .db
        .add_upload(&UploadRow {
            id: 0,
            source,
            source_id,
            file,
            room_id,
            uid: String::new(),
            cover: String::new(),
            profile: profile.to_string(),
            danmu: false,
            status: "draft".to_string(),
            message: String::new(),
            bvid: String::new(),
            created_at: now.clone(),
            updated_at: now,
        })
        .await?;
    if upload.source == "file" {
        match capture_file_cover(&state, &upload).await {
            Ok(cover) => {
                state
                    .db
                    .set_upload_cover(upload.id, &cover, &upload.updated_at)
                    .await?;
                upload.cover = cover;
            }
            Err(e) => log::warn!("Failed to capture the cover of {}: {e}", upload.file),
        }
    }
    Ok(upload_view(&state, upload).await)
}

/// Save the draft's account and upload form.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_upload(
    state: state_type!(),
    id: i64,
    uid: String,
    profile: serde_json::Value,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    if state
        .db
        .update_upload_form(id, &uid, &profile.to_string(), &now)
        .await?
    {
        Ok(())
    } else {
        Err(not_editable(&state, id).await)
    }
}

/// Replace the cover with the base64 data URL `cover`.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_upload_cover(
    state: state_type!(),
    id: i64,
    cover: String,
) -> Result<UploadView, String> {
    ensure_editable(&state.db.get_upload(id).await?)?;
    let relative = format!("{}/cover.jpg", upload_dir(id));
    write_cover(&output_dir(&state).await.join(&relative), &cover).await?;
    let now = Local::now().to_rfc3339();
    if !state.db.set_upload_cover(id, &relative, &now).await? {
        return Err(not_editable(&state, id).await);
    }
    Ok(upload_view(&state, state.db.get_upload(id).await?).await)
}

/// Remove the record; a posted video stays on Bilibili.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn delete_upload(state: state_type!(), id: i64) -> Result<(), String> {
    if !state.db.delete_upload(id).await? {
        return Err("投稿进行中，暂时无法删除".to_string());
    }
    let directory = output_dir(&state).await.join(upload_dir(id));
    if directory.exists() {
        if let Err(e) = tokio::fs::remove_dir_all(&directory).await {
            log::warn!("删除投稿目录失败: {} - {e}", directory.display());
        }
    }
    Ok(())
}

/// Post the upload in the background, exporting a project first when its
/// export is missing or stale. `subtitle_ass` renders the project's subtitles.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn post_upload(
    state: state_type!(),
    event_id: String,
    id: i64,
    subtitle_ass: Option<String>,
) -> Result<(), String> {
    let upload = state.db.get_upload(id).await?;
    ensure_editable(&upload)?;
    ready_to_post(&upload)?;
    // Concurrent requests can all pass the checks above; only one claims it.
    let now = Local::now().to_rfc3339();
    if !state.db.claim_upload(id, &now).await? {
        return Err("投稿进行中，暂时无法修改".to_string());
    }
    // Post it as claimed: edits saved before the claim are in, later ones are
    // refused.
    let upload = state.db.get_upload(id).await?;
    let profile = match ready_to_post(&upload) {
        Ok(profile) => profile,
        Err(e) => {
            state.db.mark_upload_failed(id, &e, &now).await?;
            return Err(e);
        }
    };
    let task = TaskRow {
        id: event_id.clone(),
        task_type: "upload".to_string(),
        status: "pending".to_string(),
        message: String::new(),
        metadata: json!({ "upload_id": id, "title": profile.title }).to_string(),
        created_at: Utc::now().to_rfc3339(),
    };
    let reporter = match state.db.add_task(&task).await {
        Ok(()) => ProgressReporter::new(state.db.clone(), &state.events, &event_id).await,
        Err(e) => Err(e.to_string()),
    };
    let reporter = match reporter {
        Ok(reporter) => reporter,
        Err(e) => {
            // Never leave it claimed with nothing running.
            let now = Local::now().to_rfc3339();
            state.db.mark_upload_failed(id, &e, &now).await?;
            return Err(e);
        }
    };

    #[cfg(feature = "gui")]
    let state = (*state).clone();
    tokio::spawn(async move {
        let result =
            post_upload_inner(&state, &reporter, &event_id, &upload, profile, subtitle_ass).await;
        if let Err(e) = finish_upload(&state, &reporter, &event_id, &upload, result).await {
            log::error!("Failed to record the result of upload {}: {e}", upload.id);
        }
    });
    Ok(())
}

/// Bring up the main window to start an upload of a `material` or `project`.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn request_upload(state: state_type!(), source: String, id: i64) -> Result<(), String> {
    if !matches!(source.as_str(), "material" | "project") {
        return Err(format!("未知的投稿来源: {source}"));
    }
    state
        .events
        .publish(AppEvent::UploadRequested { source, id });
    #[cfg(feature = "gui")]
    {
        use tauri::Manager;
        if let Some(window) = state.app_handle.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }
    Ok(())
}

/// The rendered video once its queued render finishes.
async fn wait_for_render(
    state: &State,
    reporter: &ProgressReporter,
    upload: &UploadRow,
    (file, rendering): (PathBuf, Option<oneshot::Receiver<Result<(), String>>>),
) -> Result<PathBuf, String> {
    let Some(done) = rendering else {
        return Ok(file);
    };
    set_upload_status(state, upload.id, "exporting").await?;
    reporter.update("等待生成视频").await;
    match done.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(format!("生成视频失败: {e}")),
        Err(_) => return Err("生成视频已取消".to_string()),
    }
    set_upload_status(state, upload.id, "posting").await?;
    Ok(file)
}

/// Turn burning danmaku into the posted video on or off. A project's setting
/// is its own, shared with its editor.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_upload_danmu(
    state: state_type!(),
    id: i64,
    danmu: bool,
) -> Result<UploadView, String> {
    let upload = state.db.get_upload(id).await?;
    if upload.source == "file" {
        return Err("视频文件没有可压制的弹幕".to_string());
    }
    // Only while it is still editable: once claimed for posting, it is refused.
    let now = Local::now().to_rfc3339();
    if !state.db.set_upload_danmu(id, danmu, &now).await? {
        return Err(not_editable(&state, id).await);
    }
    if upload.source == "project" {
        set_project_danmu(&state, upload.source_id, danmu).await?;
    }
    Ok(upload_view(&state, state.db.get_upload(id).await?).await)
}

/// The form to post, if the upload has what posting needs.
fn ready_to_post(upload: &UploadRow) -> Result<Profile, String> {
    if upload.uid.is_empty() {
        return Err("请先选择投稿账号".to_string());
    }
    let profile: Profile =
        serde_json::from_str(&upload.profile).map_err(|e| format!("投稿信息无效: {e}"))?;
    if profile.title.trim().is_empty() {
        return Err("请填写投稿标题".to_string());
    }
    Ok(profile)
}

/// Why an edit of the upload did not apply.
async fn not_editable(state: &State, id: i64) -> String {
    match state.db.get_upload(id).await {
        Ok(upload) => ensure_editable(&upload)
            .err()
            .unwrap_or_else(|| "投稿已被修改，请重试".to_string()),
        Err(_) => "投稿已被删除".to_string(),
    }
}

async fn set_upload_status(state: &State, id: i64, status: &str) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    Ok(state.db.set_upload_progress(id, status, &now).await?)
}

async fn post_upload_inner(
    state: &State,
    reporter: &ProgressReporter,
    event_id: &str,
    upload: &UploadRow,
    profile: Profile,
    subtitle_ass: Option<String>,
) -> Result<PostedVideo, String> {
    let output = output_dir(state).await;
    let (file, source_cover) = match upload.source.as_str() {
        "material" => {
            let material = state
                .db
                .get_video(upload.source_id)
                .await
                .map_err(|_| "素材已被删除".to_string())?;
            let file = output.join(&material.file);
            let cover = file.with_extension("jpg");
            // Without saved danmaku there is nothing to burn in.
            if upload.danmu && crate::material::danmu_path(&file).is_file() {
                let rendering = material_danmu_video(
                    state,
                    &format!("{event_id}-export"),
                    material.id,
                    &output.join(upload_dir(upload.id)),
                )
                .await?;
                (
                    wait_for_render(state, reporter, upload, rendering).await?,
                    Some(cover),
                )
            } else {
                (file, Some(cover))
            }
        }
        "project" => {
            let project = state
                .db
                .get_project(upload.source_id)
                .await
                .map_err(|_| "切片已被删除".to_string())?;
            let rendering = project_upload_video(
                state,
                &format!("{event_id}-export"),
                project.id,
                subtitle_ass,
                &output.join(upload_dir(upload.id)),
            )
            .await?;
            let file = wait_for_render(state, reporter, upload, rendering).await?;
            let cover = if upload.cover.is_empty() {
                Some(project_cover_file(state, &project).await?)
            } else {
                None
            };
            (file, cover)
        }
        _ => (PathBuf::from(&upload.file), None),
    };
    if !file.is_file() {
        return Err(format!("视频文件不存在: {}", file.display()));
    }
    let cover = if upload.cover.is_empty() {
        source_cover.unwrap_or_default()
    } else {
        output.join(&upload.cover)
    };
    if !cover.is_file() {
        return Err("没有可用的封面，请先设置封面".to_string());
    }
    upload_video_file(
        state,
        reporter,
        &upload.uid,
        &upload.room_id,
        &file,
        &cover,
        profile,
    )
    .await
}

async fn finish_upload(
    state: &State,
    reporter: &ProgressReporter,
    event_id: &str,
    upload: &UploadRow,
    result: Result<PostedVideo, String>,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    match result {
        Ok(posted) => {
            let profile =
                serde_json::to_string(&posted.profile).unwrap_or_else(|_| upload.profile.clone());
            state
                .db
                .mark_upload_posted(upload.id, &posted.bvid, &profile, &now)
                .await?;
            mark_source_posted(state, upload, &posted.bvid).await;
            // A retry needs the rendered video; a posted upload does not.
            remove_upload_renders(&output_dir(state).await.join(upload_dir(upload.id))).await;
            state
                .db
                .update_task(event_id, "success", "投稿成功", None)
                .await?;
        }
        Err(e) => {
            reporter.finish(false, &format!("投稿失败: {e}")).await;
            state.db.mark_upload_failed(upload.id, &e, &now).await?;
            state
                .db
                .update_task(event_id, "failed", &format!("投稿失败: {e}"), None)
                .await?;
        }
    }
    Ok(())
}

/// Show the BV id on the material or project too.
async fn mark_source_posted(state: &State, upload: &UploadRow, bvid: &str) {
    let result = match upload.source.as_str() {
        "material" => match state.db.get_video(upload.source_id).await {
            Ok(mut material) => {
                material.status = 1;
                material.bvid = bvid.to_string();
                state.db.update_video(&material).await
            }
            Err(e) => Err(e),
        },
        "project" => state.db.mark_project_posted(upload.source_id, bvid).await,
        _ => Ok(()),
    };
    if let Err(e) = result {
        log::warn!(
            "Failed to mark the source of upload {} posted: {e}",
            upload.id
        );
    }
}
