//! Commands for materials (素材) and projects (切片).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use chrono::{Local, Utc};
use recorder::danmu::DanmuEntry;
use serde::Serialize;
use serde_json::json;
use tokio::sync::oneshot;

use crate::danmu2ass::Danmu2AssOptions;
use crate::database::project::ProjectRow;
use crate::database::task::TaskRow;
use crate::database::upload::UploadRow;
use crate::database::video::VideoRow;
use crate::database::Database;
use crate::ffmpeg::project::{render_project, ProjectRender, RenderClip};
use crate::ffmpeg::runner::TempArtifact;
use crate::handlers::subtitle_fonts::prepare_subtitle_font_directory;
use crate::handlers::utils::get_disk_info_inner;
use crate::handlers::video::{
    ass_primary_font_name, auto_subtitle_material, create_material, ensure_material_duration,
    notify_materials_created, run_queued, validate_ass_document,
};
use crate::material::Peaks;
use crate::progress::progress_reporter::{ProgressReporter, ProgressReporterTrait};
use crate::project::{srt_cues, Timeline, TimelineClip};
use crate::recorder_manager::ClipRangeParams;
use crate::state::State;
use crate::state_type;
use crate::task::{Task, TaskPriority};

#[cfg(feature = "gui")]
use tauri::State as TauriState;

/// Directory of a project's own files, relative to the output directory.
fn project_dir(id: i64) -> String {
    format!("projects/{id}")
}

/// Where web clients' exports wait to be downloaded, relative to the cache.
fn web_export_dir(id: i64) -> String {
    format!("exports/{id}")
}

/// A project with what the lists and the editor show about it.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectView {
    #[serde(flatten)]
    pub project: ProjectRow,
    pub duration: f64,
    pub clip_count: usize,
    /// Cover to show, relative to the output directory: the project's own or
    /// the first material's thumbnail.
    pub display_cover: String,
    /// False when the project is one untouched material, which is exported
    /// and uploaded as is.
    pub needs_render: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectBrief {
    pub id: i64,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GeneratedMaterials {
    pub materials: Vec<VideoRow>,
    pub project: Option<ProjectView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportStarted {
    pub task: TaskRow,
    /// The file being written.
    pub path: String,
    /// Where the web client downloads it, relative to the cache directory.
    pub download: Option<String>,
}

async fn output_dir(state: &State) -> PathBuf {
    PathBuf::from(&state.config.read().await.output)
}

/// The danmaku style now, kept with an export so its render matches its hash.
async fn danmu_options(state: &State) -> Danmu2AssOptions {
    state.config.read().await.danmu_ass_options.clone()
}

fn material_duration(video: &VideoRow) -> f64 {
    if video.duration > 0.0 {
        video.duration
    } else {
        video.length as f64
    }
}

async fn materials_by_id(state: &State, ids: &[i64]) -> Result<HashMap<i64, VideoRow>, String> {
    let mut materials = HashMap::new();
    for &id in ids {
        if let Ok(video) = state.db.get_video(id).await {
            materials.insert(id, ensure_material_duration(state, video).await);
        }
    }
    Ok(materials)
}

/// The material a project is exported and uploaded as: one untouched material
/// without subtitles or danmaku, which needs no rendering.
fn plain_material<'a>(
    timeline: &Timeline,
    materials: &'a HashMap<i64, VideoRow>,
) -> Option<&'a VideoRow> {
    let material = materials.get(&timeline.clips.first()?.material_id)?;
    timeline
        .single_full_material(material_duration(material))
        .map(|_| material)
}

fn project_view(project: ProjectRow, materials: &HashMap<i64, VideoRow>) -> ProjectView {
    let timeline = Timeline::parse(&project.timeline).unwrap_or_default();
    let display_cover = if project.cover.is_empty() {
        timeline
            .clips
            .first()
            .and_then(|clip| materials.get(&clip.material_id))
            .map(|material| material.cover.clone())
            .unwrap_or_default()
    } else {
        project.cover.clone()
    };
    ProjectView {
        needs_render: plain_material(&timeline, materials).is_none(),
        duration: timeline.duration(),
        clip_count: timeline.clips.len(),
        display_cover,
        project,
    }
}

pub(crate) async fn single_project_view(
    state: &State,
    project: ProjectRow,
) -> Result<ProjectView, String> {
    let timeline = Timeline::parse(&project.timeline).unwrap_or_default();
    let materials = materials_by_id(state, &timeline.material_ids()).await?;
    Ok(project_view(project, &materials))
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_projects(state: state_type!()) -> Result<Vec<ProjectView>, String> {
    let projects = state.db.get_projects().await?;
    // Avoid probing every material for the list; durations are filled when
    // materials are created or opened.
    let materials: HashMap<i64, VideoRow> = state
        .db
        .get_all_videos()
        .await?
        .into_iter()
        .map(|video| (video.id, video))
        .collect();
    let mut views = Vec::with_capacity(projects.len());
    for project in projects {
        views.push(project_view(project, &materials));
    }
    Ok(views)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_project(state: state_type!(), id: i64) -> Result<ProjectView, String> {
    let project = state.db.get_project(id).await?;
    single_project_view(&state, project).await
}

/// Anchor each clip to the cached subtitles of its material.
async fn import_material_subtitles(
    output: &Path,
    timeline: &mut Timeline,
    materials: &HashMap<i64, VideoRow>,
    only_material: Option<i64>,
) {
    let clips: Vec<(String, i64)> = timeline
        .clips
        .iter()
        .filter(|clip| only_material.is_none_or(|id| id == clip.material_id))
        .filter(|clip| !timeline.subtitles.iter().any(|s| s.clip_id == clip.id))
        .map(|clip| (clip.id.clone(), clip.material_id))
        .collect();
    for (clip_id, material_id) in clips {
        let Some(material) = materials.get(&material_id) else {
            continue;
        };
        let srt = output.join(&material.file).with_extension("srt");
        if let Ok(content) = tokio::fs::read_to_string(&srt).await {
            timeline.set_clip_subtitles(&clip_id, srt_cues(&content));
        }
    }
}

/// Save the base64 data URL `cover` as the image file `path`.
pub(crate) async fn write_cover(path: &Path, cover: &str) -> Result<(), String> {
    let base64 = cover
        .split("base64,")
        .nth(1)
        .ok_or_else(|| "Cover is not a base64 data URL".to_string())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64)
        .map_err(|e| format!("Failed to decode cover: {e}"))?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    tokio::fs::write(path, bytes)
        .await
        .map_err(|e| e.to_string())
}

async fn write_project_cover(
    output: &Path,
    project_id: i64,
    cover: &str,
) -> Result<String, String> {
    let relative = format!("{}/cover.jpg", project_dir(project_id));
    write_cover(&output.join(&relative), cover).await?;
    Ok(relative)
}

struct NewProject<'a> {
    title: String,
    room_id: String,
    platform: String,
    materials: &'a [VideoRow],
    cover: Option<&'a str>,
}

async fn create_project_inner(state: &State, new: NewProject<'_>) -> Result<ProjectView, String> {
    let first = new.materials.first();
    let title = if !new.title.trim().is_empty() {
        new.title.trim().to_string()
    } else if let Some(first) = first {
        material_title(first)
    } else {
        "未命名切片".to_string()
    };
    let room_id = if new.room_id.is_empty() {
        first.map(|m| m.room_id.clone()).unwrap_or_default()
    } else {
        new.room_id
    };
    let platform = if new.platform.is_empty() {
        first.map(|m| m.platform.clone()).unwrap_or_default()
    } else {
        new.platform
    };
    let mut timeline = Timeline::from_materials(
        &new.materials
            .iter()
            .map(|m| (m.id, material_duration(m)))
            .collect::<Vec<_>>(),
    );
    let output = output_dir(state).await;
    let materials: HashMap<i64, VideoRow> =
        new.materials.iter().map(|m| (m.id, m.clone())).collect();
    import_material_subtitles(&output, &mut timeline, &materials, None).await;
    let now = Local::now().to_rfc3339();
    let mut project = state
        .db
        .add_project(&ProjectRow {
            id: 0,
            room_id,
            platform,
            title,
            note: String::new(),
            cover: String::new(),
            timeline: timeline.to_json(),
            profile: String::new(),
            status: 0,
            bvid: String::new(),
            created_at: now.clone(),
            updated_at: now,
        })
        .await?;
    if let Some(cover) = new.cover.filter(|cover| !cover.is_empty()) {
        match write_project_cover(&output, project.id, cover).await {
            Ok(relative) => {
                state.db.set_project_cover(project.id, &relative).await?;
                project.cover = relative;
            }
            Err(e) => log::warn!("Failed to save project cover: {e}"),
        }
    }
    Ok(project_view(project, &materials))
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn create_project(
    state: state_type!(),
    title: String,
    room_id: String,
    platform: String,
    material_ids: Vec<i64>,
) -> Result<ProjectView, String> {
    let materials = materials_by_id(&state, &material_ids).await?;
    let materials: Vec<VideoRow> = material_ids
        .iter()
        .filter_map(|id| materials.get(id).cloned())
        .collect();
    create_project_inner(
        &state,
        NewProject {
            title,
            room_id,
            platform,
            materials: &materials,
            cover: None,
        },
    )
    .await
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_project_info(
    state: state_type!(),
    id: i64,
    title: String,
    note: String,
) -> Result<(), String> {
    let now = Local::now().to_rfc3339();
    Ok(state
        .db
        .update_project_info(id, &title, &note, &now)
        .await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_project_timeline(
    state: state_type!(),
    id: i64,
    timeline: Timeline,
) -> Result<ProjectView, String> {
    let now = Local::now().to_rfc3339();
    state
        .db
        .set_project_timeline(id, &timeline.to_json(), &now)
        .await?;
    let project = state.db.get_project(id).await?;
    single_project_view(&state, project).await
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn delete_project(state: state_type!(), id: i64) -> Result<(), String> {
    state.db.delete_project(id).await?;
    let cache = PathBuf::from(&state.config.read().await.cache);
    for directory in [
        output_dir(&state).await.join(project_dir(id)),
        cache.join(web_export_dir(id)),
    ] {
        if directory.exists() {
            if let Err(e) = tokio::fs::remove_dir_all(&directory).await {
                log::warn!("删除切片目录失败: {} - {e}", directory.display());
            }
        }
    }
    Ok(())
}

/// Projects that use the material.
async fn projects_using(db: &Database, material_id: i64) -> Result<Vec<ProjectRow>, String> {
    Ok(db
        .get_projects()
        .await?
        .into_iter()
        .filter(|project| {
            Timeline::parse(&project.timeline).is_ok_and(|t| t.uses_material(material_id))
        })
        .collect())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_material_usage(
    state: state_type!(),
    id: i64,
) -> Result<Vec<ProjectBrief>, String> {
    Ok(projects_using(&state.db, id)
        .await?
        .into_iter()
        .map(|project| ProjectBrief {
            id: project.id,
            title: project.title,
        })
        .collect())
}

/// Take a material that is about to be deleted out of every project.
pub(crate) async fn remove_material_from_projects(
    state: &State,
    material_id: i64,
) -> Result<(), String> {
    for project in projects_using(&state.db, material_id).await? {
        edit_project_timeline(state, project.id, |timeline| {
            timeline.remove_material(material_id)
        })
        .await?;
    }
    Ok(())
}

/// Attempts at an edit before giving up on a timeline that keeps changing.
const TIMELINE_EDIT_ATTEMPTS: usize = 5;

/// Apply `edit`, which tells whether it changed anything, to the project's
/// saved timeline. The write only replaces the timeline it was made from, so
/// a save from the editor in between is edited again instead of overwritten.
async fn edit_project_timeline(
    state: &State,
    id: i64,
    mut edit: impl FnMut(&mut Timeline) -> bool,
) -> Result<(), String> {
    for _ in 0..TIMELINE_EDIT_ATTEMPTS {
        let saved = state.db.get_project(id).await?.timeline;
        let mut timeline = Timeline::parse(&saved)?;
        if !edit(&mut timeline) {
            return Ok(());
        }
        let now = Local::now().to_rfc3339();
        if state
            .db
            .replace_project_timeline(id, &saved, &timeline.to_json(), &now)
            .await?
        {
            return Ok(());
        }
    }
    Err("切片正在被修改，请稍后重试".to_string())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_material_peaks(state: state_type!(), id: i64) -> Result<Peaks, String> {
    let video = state.db.get_video(id).await?;
    let path = output_dir(&state).await.join(&video.file);
    crate::material::peaks(&path).await
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_material_danmu(state: state_type!(), id: i64) -> Result<Vec<DanmuEntry>, String> {
    let video = state.db.get_video(id).await?;
    let path = output_dir(&state).await.join(&video.file);
    Ok(crate::material::load_danmus(&path).await)
}

async fn check_free_space(state: &State) -> Result<(), String> {
    let mut output = output_dir(state).await;
    if output.is_relative() {
        let cwd =
            std::env::current_dir().map_err(|e| format!("Failed to get current directory: {e}"))?;
        output = cwd.join(output);
    }
    if let Ok(disk_info) = get_disk_info_inner(output).await {
        // Preserve 1GB for other usage.
        if disk_info.free < 1024 * 1024 * 1024 {
            return Err("存储空间不足（少于 1GB），已取消".to_string());
        }
    }
    Ok(())
}

/// Cut each range of a recording into its own material, optionally creating
/// a project that plays them in order.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn generate_materials(
    state: state_type!(),
    event_id: String,
    params: ClipRangeParams,
    create_project: bool,
) -> Result<GeneratedMaterials, String> {
    check_free_space(&state).await?;
    let reporter = ProgressReporter::new(state.db.clone(), &state.events, &event_id).await?;
    let mut params_without_cover = params.clone();
    params_without_cover.cover = String::new();
    state
        .db
        .add_task(&TaskRow {
            id: event_id.clone(),
            task_type: "generate_materials".to_string(),
            status: "pending".to_string(),
            message: String::new(),
            metadata: json!({
                "params": params_without_cover,
                "create_project": create_project,
            })
            .to_string(),
            created_at: Utc::now().to_rfc3339(),
        })
        .await?;

    #[cfg(feature = "gui")]
    let state_clone = (*state).clone();
    #[cfg(feature = "headless")]
    let state_clone = state.clone();
    let task_id = event_id.clone();
    run_queued(
        &state,
        &event_id,
        async move {
            let result =
                generate_materials_inner(&state_clone, &reporter, &task_id, params, create_project)
                    .await;
            let (status, message) = match &result {
                Ok(generated) => (
                    "success",
                    format!("已生成 {} 个素材", generated.materials.len()),
                ),
                Err(e) => ("failed", format!("生成素材失败: {e}")),
            };
            reporter.finish(result.is_ok(), &message).await;
            let _ = state_clone
                .db
                .update_task(&task_id, status, &message, None)
                .await;
            result
        },
        "生成素材任务已取消",
        "生成素材任务失败",
    )
    .await
}

async fn generate_materials_inner(
    state: &State,
    reporter: &ProgressReporter,
    task_id: &str,
    params: ClipRangeParams,
    create_project: bool,
) -> Result<GeneratedMaterials, String> {
    // No ranges means the whole recording, as for `clip_range`.
    let ranges = if params.ranges.is_empty() {
        vec![None]
    } else {
        params.ranges.iter().cloned().map(Some).collect()
    };
    let count = ranges.len();
    let mut materials = Vec::with_capacity(count);
    for (index, range) in ranges.into_iter().enumerate() {
        let mut material_params = params.clone();
        material_params.ranges = range.into_iter().collect();
        material_params.transition = None;
        // A captured frame only represents a single range.
        if count > 1 {
            material_params.cover = String::new();
        }
        let stage = if count > 1 {
            reporter.stage(&format!("生成素材 {}/{count}", index + 1))
        } else {
            reporter.clone()
        };
        materials.push(create_material(state, &stage, &material_params).await?);
    }
    notify_materials_created(state, &params.room_id, &materials).await?;

    let project = if create_project {
        Some(
            create_project_inner(
                state,
                NewProject {
                    title: params.title.clone(),
                    room_id: params.room_id.clone(),
                    platform: params.platform.clone(),
                    materials: &materials,
                    cover: Some(&params.cover),
                },
            )
            .await?,
        )
    } else {
        None
    };

    for material in &materials {
        if auto_subtitle_material(state, task_id, material.id).await {
            if let Some(project) = &project {
                attach_material_subtitles(state, project.project.id, material).await?;
            }
        }
    }

    Ok(GeneratedMaterials { materials, project })
}

/// Give the material's clips in a project its freshly generated subtitles.
async fn attach_material_subtitles(
    state: &State,
    project_id: i64,
    material: &VideoRow,
) -> Result<(), String> {
    let srt = output_dir(state)
        .await
        .join(&material.file)
        .with_extension("srt");
    let Ok(content) = tokio::fs::read_to_string(&srt).await else {
        return Ok(());
    };
    let cues = srt_cues(&content);
    edit_project_timeline(state, project_id, |timeline| {
        let clips: Vec<String> = timeline
            .clips
            .iter()
            .filter(|clip| clip.material_id == material.id)
            .filter(|clip| !timeline.subtitles.iter().any(|s| s.clip_id == clip.id))
            .map(|clip| clip.id.clone())
            .collect();
        for clip_id in &clips {
            timeline.set_clip_subtitles(clip_id, cues.clone());
        }
        !clips.is_empty()
    })
    .await
}

/// The video to upload for a project: its plain material, or a render of its
/// timeline kept in `directory` until the upload succeeds. While that render is
/// queued, the receiver yields its result.
pub(crate) async fn project_upload_video(
    state: &State,
    event_id: &str,
    id: i64,
    subtitle_ass: Option<String>,
    directory: &Path,
) -> Result<(PathBuf, Option<oneshot::Receiver<Result<(), String>>>), String> {
    let project = state.db.get_project(id).await?;
    let timeline = Timeline::parse(&project.timeline)?;
    if timeline.clips.is_empty() {
        return Err("切片没有片段".to_string());
    }
    let materials = materials_by_id(state, &timeline.material_ids()).await?;
    timeline.validate(|id| materials.get(&id).map(material_duration))?;
    if let Some(material) = plain_material(&timeline, &materials) {
        return Ok((output_dir(state).await.join(&material.file), None));
    }
    if timeline.has_visible_subtitles() && subtitle_ass.is_none() {
        return Err("切片有字幕，但没有提供字幕样式".to_string());
    }
    let source = ExportSource {
        task_type: "export_project",
        title: project.title.clone(),
        metadata: json!({ "project_id": project.id, "title": project.title }),
        subtitle_ass: subtitle_ass.filter(|_| timeline.has_visible_subtitles()),
        timeline,
        materials: &materials,
        danmu_options: danmu_options(state).await,
    };
    upload_render(state, event_id, source, directory).await
}

/// The video to upload for a material with its danmaku burned in, rendered as
/// a project of the whole material and kept in `directory` like a project's.
pub(crate) async fn material_danmu_video(
    state: &State,
    event_id: &str,
    id: i64,
    directory: &Path,
) -> Result<(PathBuf, Option<oneshot::Receiver<Result<(), String>>>), String> {
    let materials = materials_by_id(state, &[id]).await?;
    let material = materials
        .get(&id)
        .ok_or_else(|| "素材已被删除".to_string())?;
    let duration = material_duration(material);
    if duration <= 0.0 {
        return Err("无法读取素材时长".to_string());
    }
    let title = material_title(material);
    let source = ExportSource {
        task_type: "render_material_danmu",
        metadata: json!({ "video_id": id, "title": title }),
        title,
        // A fixed clip id keeps the render hash, and so a retry's reuse, stable.
        timeline: Timeline {
            clips: vec![TimelineClip {
                id: "material".to_string(),
                material_id: id,
                start: 0.0,
                end: duration,
            }],
            danmu: true,
            ..Timeline::default()
        },
        materials: &materials,
        subtitle_ass: None,
        danmu_options: danmu_options(state).await,
    };
    upload_render(state, event_id, source, directory).await
}

/// Render `source` into `directory` for an upload. A retry after a failed
/// upload reuses the render when nothing changed.
async fn upload_render(
    state: &State,
    event_id: &str,
    source: ExportSource<'_>,
    directory: &Path,
) -> Result<(PathBuf, Option<oneshot::Receiver<Result<(), String>>>), String> {
    let hash = source.render_hash();
    let target = directory.join(format!("{UPLOAD_RENDER_PREFIX}{hash}.mp4"));
    if target.is_file() {
        return Ok((target, None));
    }
    remove_upload_renders(directory).await;
    let (_, done) = queue_export(
        state,
        event_id,
        source,
        ExportTarget::Upload(target.clone()),
    )
    .await?;
    Ok((target, Some(done)))
}

/// Turn burning danmaku into the project's exports and uploads on or off.
pub(crate) async fn set_project_danmu(state: &State, id: i64, danmu: bool) -> Result<(), String> {
    edit_project_timeline(state, id, |timeline| {
        let changed = timeline.danmu != danmu;
        timeline.danmu = danmu;
        changed
    })
    .await
}

/// A material's title, or its file name without the extension.
pub(crate) fn material_title(material: &VideoRow) -> String {
    if material.title.is_empty() {
        Path::new(&material.file)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string()
    } else {
        material.title.clone()
    }
}

const UPLOAD_RENDER_PREFIX: &str = "render_";

/// Delete the videos rendered for an upload in `directory`.
pub(crate) async fn remove_upload_renders(directory: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(directory).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(UPLOAD_RENDER_PREFIX)
        {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

/// The project's cover, or the first material's thumbnail.
pub(crate) async fn project_cover_file(
    state: &State,
    project: &ProjectRow,
) -> Result<PathBuf, String> {
    let output = output_dir(state).await;
    if !project.cover.is_empty() {
        return Ok(output.join(&project.cover));
    }
    let timeline = Timeline::parse(&project.timeline)?;
    let first = timeline
        .clips
        .first()
        .ok_or_else(|| "切片没有片段".to_string())?;
    let material = state.db.get_video(first.material_id).await?;
    let path = output.join(&material.file).with_extension("jpg");
    if path.exists() {
        Ok(path)
    } else {
        Err("切片没有封面，请先设置封面".to_string())
    }
}

/// Where an export is written.
enum ExportTarget {
    /// A file the user asked for, who is told when it is done.
    User(PathBuf),
    /// A video rendered for an upload, which reports its own result.
    Upload(PathBuf),
}

impl ExportTarget {
    fn path(&self) -> &Path {
        match self {
            Self::User(path) | Self::Upload(path) => path,
        }
    }
}

/// A timeline to export over its materials, and how its task is listed.
struct ExportSource<'a> {
    task_type: &'static str,
    /// Shown in the task list and messages.
    title: String,
    /// Task metadata naming what is exported.
    metadata: serde_json::Value,
    timeline: Timeline,
    materials: &'a HashMap<i64, VideoRow>,
    subtitle_ass: Option<String>,
    danmu_options: Danmu2AssOptions,
}

impl ExportSource<'_> {
    /// Identifies what rendering this source contains.
    fn render_hash(&self) -> String {
        let mut extra = self.subtitle_ass.clone().unwrap_or_default();
        if self.timeline.danmu {
            extra.push_str(&serde_json::to_string(&self.danmu_options).unwrap_or_default());
        }
        self.timeline.render_hash(&extra)
    }
}

/// Everything an export needs, fixed when it is queued so that later edits
/// cannot change a waiting job.
struct ExportJob {
    title: String,
    /// The material to copy instead of rendering.
    plain: Option<PathBuf>,
    timeline: Timeline,
    /// Each material's file, by id.
    files: HashMap<i64, PathBuf>,
    subtitle_ass: Option<String>,
    font_name: Option<String>,
    danmu_options: Danmu2AssOptions,
    cache: PathBuf,
    target: ExportTarget,
}

/// Export the timeline to `target`, a file the user picked, or without one to
/// the cache for the web client to download.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn export_project(
    state: state_type!(),
    event_id: String,
    id: i64,
    timeline: Timeline,
    subtitle_ass: Option<String>,
    target: Option<String>,
) -> Result<ExportStarted, String> {
    let project = state.db.get_project(id).await?;
    if timeline.clips.is_empty() {
        return Err("时间轴上还没有片段".to_string());
    }
    let materials = materials_by_id(&state, &timeline.material_ids()).await?;
    timeline.validate(|id| materials.get(&id).map(material_duration))?;
    let now = Local::now().to_rfc3339();
    state
        .db
        .set_project_timeline(id, &timeline.to_json(), &now)
        .await?;

    let (path, download) = match target {
        Some(path) => (mp4_path(PathBuf::from(path)), None),
        None => {
            let relative = format!("{}/export.mp4", web_export_dir(id));
            let cache = PathBuf::from(&state.config.read().await.cache);
            (cache.join(&relative), Some(relative))
        }
    };
    let output = output_dir(&state).await;
    if materials
        .values()
        .any(|material| output.join(&material.file) == path)
    {
        return Err("不能覆盖切片使用的素材文件".to_string());
    }
    let source = ExportSource {
        task_type: "export_project",
        title: project.title.clone(),
        metadata: json!({ "project_id": project.id, "title": project.title }),
        timeline,
        materials: &materials,
        subtitle_ass,
        danmu_options: danmu_options(&state).await,
    };
    let (task, _) =
        queue_export(&state, &event_id, source, ExportTarget::User(path.clone())).await?;
    Ok(ExportStarted {
        task,
        path: path.to_string_lossy().into_owned(),
        download,
    })
}

/// `path` with the `.mp4` extension the encoder writes.
fn mp4_path(path: PathBuf) -> PathBuf {
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
    {
        path
    } else {
        let mut name = path.into_os_string();
        name.push(".mp4");
        PathBuf::from(name)
    }
}

/// Copy the plain material or render the timeline to `target` on the task
/// queue. The receiver yields the result, or closes if the export is cancelled.
async fn queue_export(
    state: &State,
    event_id: &str,
    source: ExportSource<'_>,
    target: ExportTarget,
) -> Result<(TaskRow, oneshot::Receiver<Result<(), String>>), String> {
    let ExportSource {
        task_type,
        title,
        metadata,
        timeline,
        materials,
        subtitle_ass,
        danmu_options,
    } = source;
    let output = output_dir(state).await;
    let plain = plain_material(&timeline, materials).map(|material| output.join(&material.file));
    let subtitle_ass = subtitle_ass.filter(|_| timeline.has_visible_subtitles());
    if let Some(ass) = &subtitle_ass {
        validate_ass_document(ass)?;
    }
    let font_name = subtitle_ass
        .as_deref()
        .map(ass_primary_font_name)
        .transpose()?;
    check_free_space(state).await?;

    let job = ExportJob {
        title,
        plain,
        files: materials
            .iter()
            .map(|(id, material)| (*id, output.join(&material.file)))
            .collect(),
        timeline,
        subtitle_ass,
        font_name,
        danmu_options,
        cache: PathBuf::from(&state.config.read().await.cache),
        target,
    };
    let task = TaskRow {
        id: event_id.to_string(),
        task_type: task_type.to_string(),
        status: "pending".to_string(),
        message: "等待导出".to_string(),
        metadata: metadata.to_string(),
        created_at: Utc::now().to_rfc3339(),
    };
    state.db.add_task(&task).await?;
    let reporter = ProgressReporter::new(state.db.clone(), &state.events, event_id).await?;

    let state_clone = state.clone();
    let task_id = event_id.to_string();
    let (done_tx, done) = oneshot::channel();
    let enqueue_result = state
        .task_manager
        .add_task(Task::new(
            task_id.clone(),
            TaskPriority::Normal,
            async move {
                reporter.update("准备导出").await;
                let result = export_project_inner(&state_clone, &reporter, job).await;
                let (status, message) = match &result {
                    Ok(()) => ("success", "导出完成".to_string()),
                    Err(error) => ("failed", format!("导出失败: {error}")),
                };
                state_clone
                    .db
                    .update_task(&task_id, status, &message, None)
                    .await?;
                reporter.finish(result.is_ok(), &message).await;
                let _ = done_tx.send(result.clone());
                result
            },
        ))
        .await;
    if let Err(error) = enqueue_result {
        state
            .db
            .update_task(
                event_id,
                "failed",
                &format!("加入导出队列失败: {error}"),
                None,
            )
            .await?;
        return Err(error);
    }
    Ok((task, done))
}

async fn export_project_inner(
    state: &State,
    reporter: &ProgressReporter,
    job: ExportJob,
) -> Result<(), String> {
    let target = job.target.path().to_path_buf();
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    // Written beside the target and moved over it once complete, so a failed
    // export never replaces an existing file with a broken one.
    let stem = target.file_stem().unwrap_or_default().to_string_lossy();
    let staging = target.with_file_name(format!("{stem}.part.mp4"));
    match &job.plain {
        Some(material) => {
            reporter.update("复制素材文件").await;
            let guard = TempArtifact::new(&staging);
            tokio::fs::copy(material, &staging)
                .await
                .map_err(|e| format!("复制素材文件失败: {e}"))?;
            guard.keep();
        }
        None => render_timeline(reporter, &job, &staging).await?,
    }
    if let Err(e) = tokio::fs::rename(&staging, &target).await {
        let _ = tokio::fs::remove_file(&staging).await;
        return Err(format!("保存导出文件失败: {e}"));
    }
    if let ExportTarget::User(path) = &job.target {
        state
            .db
            .new_message(
                "切片导出完成",
                &format!("切片「{}」已导出到 {}", job.title, path.display()),
            )
            .await?;
    }
    Ok(())
}

async fn render_timeline(
    reporter: &ProgressReporter,
    job: &ExportJob,
    output: &Path,
) -> Result<(), String> {
    let mut clips = Vec::with_capacity(job.timeline.clips.len());
    for clip in &job.timeline.clips {
        let path = job
            .files
            .get(&clip.material_id)
            .ok_or_else(|| format!("素材 {} 不存在", clip.material_id))?;
        let metadata = ffmpeg_utils::extract_video_metadata(path).await?;
        clips.push(RenderClip {
            path: path.clone(),
            start: clip.start,
            duration: clip.duration(),
            has_audio: !metadata.audio_codec.is_empty(),
            fps: metadata.fps,
        });
    }

    let danmus = if job.timeline.danmu {
        let mut by_material = HashMap::new();
        for (id, path) in &job.files {
            by_material.insert(*id, crate::material::load_danmus(path).await);
        }
        job.timeline
            .map_danmus(|id| by_material.get(&id).map(Vec::as_slice))
    } else {
        Vec::new()
    };

    // Kept alive until FFmpeg exits; dropping it removes the directory.
    let fonts_directory = match &job.font_name {
        Some(font_name) => {
            Some(prepare_subtitle_font_directory(job.cache.clone(), font_name.clone()).await?)
        }
        None => None,
    };

    render_project(
        reporter,
        ProjectRender {
            clips,
            subtitle_ass: job.subtitle_ass.clone(),
            fonts_directory: fonts_directory.as_ref().map(|directory| directory.path()),
            danmus,
            danmu_options: job.danmu_options.clone(),
            output,
        },
    )
    .await
}

/// Give every video that existed before projects its own single-material
/// project, carrying over its title, note, upload state and subtitles.
pub async fn create_legacy_projects(db: Arc<Database>, output: PathBuf) {
    let videos = match db.get_videos_needing_project().await {
        Ok(videos) => videos,
        Err(e) => {
            log::error!("Failed to list videos for project migration: {e}");
            return;
        }
    };
    if videos.is_empty() {
        return;
    }
    log::info!("Creating projects for {} existing videos", videos.len());
    for mut video in videos {
        let path = output.join(&video.file);
        if video.duration <= 0.0 {
            if let Ok(metadata) = ffmpeg_utils::extract_video_metadata(&path).await {
                if metadata.duration > 0.0 {
                    video.duration = metadata.duration;
                    let _ = db.update_video_duration(video.id, video.duration).await;
                }
            }
        }
        let mut timeline = Timeline::from_materials(&[(video.id, material_duration(&video))]);
        let materials = HashMap::from([(video.id, video.clone())]);
        import_material_subtitles(&output, &mut timeline, &materials, None).await;
        let title = material_title(&video);
        let posted = !video.bvid.is_empty();
        let profile = if posted {
            json!({
                "title": video.title,
                "desc": video.desc,
                "tag": video.tags,
                "tid": video.area,
            })
            .to_string()
        } else {
            String::new()
        };
        let project = ProjectRow {
            id: 0,
            room_id: video.room_id.clone(),
            platform: video.platform.clone(),
            title,
            note: video.note.clone(),
            cover: String::new(),
            timeline: timeline.to_json(),
            profile,
            status: i64::from(posted),
            bvid: video.bvid.clone(),
            created_at: video.created_at.clone(),
            updated_at: video.created_at.clone(),
        };
        let upload = posted.then(|| UploadRow {
            id: 0,
            source: "project".to_string(),
            // Set to the new project's id.
            source_id: 0,
            file: String::new(),
            room_id: project.room_id.clone(),
            uid: String::new(),
            cover: String::new(),
            profile: project.profile.clone(),
            danmu: false,
            status: "posted".to_string(),
            message: String::new(),
            bvid: project.bvid.clone(),
            created_at: project.created_at.clone(),
            updated_at: project.created_at.clone(),
        });
        if let Err(e) = db.add_legacy_project(video.id, &project, upload).await {
            log::error!("Failed to create project for video {}: {e}", video.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::credentials::CredentialCipher;

    #[tokio::test]
    async fn a_video_whose_migration_fails_is_left_for_the_next_start() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for migration in crate::get_migrations() {
            sqlx::raw_sql(migration.sql).execute(&pool).await.unwrap();
        }
        sqlx::raw_sql(
            "INSERT INTO videos (room_id, cover, file, note, length, size, status, bvid, title, desc, tags, area, created_at, platform, needs_project)
             VALUES ('1', 'a.jpg', 'a.mp4', '', 30, 1, 1, 'BV1xx', 'Posted', '', '', 21, '2026-01-01T00:00:00+08:00', 'bilibili', 1);
             DROP TABLE uploads;",
        )
        .execute(&pool)
        .await
        .unwrap();
        let db = Arc::new(Database::new(
            pool,
            CredentialCipher::new(&[0; 32]).unwrap(),
            crate::events::EventBus::new(),
        ));
        let output = tempfile::tempdir().unwrap();

        // Recording its upload fails, so its project must not be kept either.
        create_legacy_projects(db.clone(), output.path().to_path_buf()).await;

        assert!(db.get_projects().await.unwrap().is_empty());
        assert_eq!(db.get_videos_needing_project().await.unwrap().len(), 1);
    }

    #[test]
    fn exports_keep_the_mp4_extension_the_encoder_writes() {
        assert_eq!(mp4_path("a/b.mp4".into()), PathBuf::from("a/b.mp4"));
        assert_eq!(mp4_path("a/b.MP4".into()), PathBuf::from("a/b.MP4"));
        assert_eq!(mp4_path("a/b".into()), PathBuf::from("a/b.mp4"));
        // A dot in the name is not an extension to replace.
        assert_eq!(mp4_path("a/v1.2".into()), PathBuf::from("a/v1.2.mp4"));
    }

    #[tokio::test]
    async fn existing_videos_get_one_project_each_with_their_upload_state_and_subtitles() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let migrations = crate::get_migrations();
        let (before, after): (Vec<_>, Vec<_>) =
            migrations.into_iter().partition(|m| m.version < 18);
        for migration in before {
            sqlx::raw_sql(migration.sql).execute(&pool).await.unwrap();
        }
        sqlx::raw_sql(
            "INSERT INTO videos (room_id, cover, file, note, length, size, status, bvid, title, desc, tags, area, created_at, platform)
             VALUES ('1', 'a.jpg', 'a.mp4', 'note', 30, 1, 1, 'BV1xx', 'Posted', 'desc', 'tag', 21, '2026-01-01T00:00:00+08:00', 'bilibili'),
                    ('1', 'b.jpg', 'b.mp4', '', 12, 1, 0, '', '', '', '', 0, '2026-01-02T00:00:00+08:00', 'bilibili');",
        )
        .execute(&pool)
        .await
        .unwrap();
        for migration in after {
            sqlx::raw_sql(migration.sql).execute(&pool).await.unwrap();
        }
        let db = Arc::new(Database::new(
            pool,
            CredentialCipher::new(&[0; 32]).unwrap(),
            crate::events::EventBus::new(),
        ));
        let output = tempfile::tempdir().unwrap();
        std::fs::write(
            output.path().join("a.srt"),
            "1\n00:00:01,000 --> 00:00:02,000\nhello\n",
        )
        .unwrap();

        create_legacy_projects(db.clone(), output.path().to_path_buf()).await;
        // Running again, as on every startup, must not duplicate projects.
        create_legacy_projects(db.clone(), output.path().to_path_buf()).await;

        let mut projects = db.get_projects().await.unwrap();
        projects.sort_by_key(|project| project.id);
        assert_eq!(projects.len(), 2);
        let posted = &projects[0];
        assert_eq!((posted.title.as_str(), posted.status), ("Posted", 1));
        assert_eq!(
            (posted.bvid.as_str(), posted.note.as_str()),
            ("BV1xx", "note")
        );
        // Its submission is listed with the other uploads.
        let uploads = db.get_uploads().await.unwrap();
        assert_eq!(uploads.len(), 1);
        assert_eq!(
            (
                uploads[0].source.as_str(),
                uploads[0].source_id,
                uploads[0].status.as_str(),
                uploads[0].bvid.as_str()
            ),
            ("project", posted.id, "posted", "BV1xx")
        );
        let timeline = Timeline::parse(&posted.timeline).unwrap();
        assert_eq!(timeline.clips.len(), 1);
        // The file cannot be probed, so the stored whole-second length is used.
        assert_eq!(
            (timeline.clips[0].start, timeline.clips[0].end),
            (0.0, 30.0)
        );
        assert_eq!(timeline.subtitles.len(), 1);
        assert_eq!(timeline.subtitles[0].clip_id, timeline.clips[0].id);
        assert_eq!(
            timeline.subtitle_span(&timeline.subtitles[0]),
            Some((1.0, 2.0))
        );
        let unposted = &projects[1];
        assert_eq!((unposted.title.as_str(), unposted.status), ("b", 0));
        assert!(Timeline::parse(&unposted.timeline)
            .unwrap()
            .subtitles
            .is_empty());

        // Videos created from now on are materials only.
        db.add_video(&VideoRow {
            id: 0,
            room_id: "1".into(),
            cover: String::new(),
            file: "c.mp4".into(),
            note: String::new(),
            length: 1,
            size: 1,
            status: 0,
            bvid: String::new(),
            title: String::new(),
            desc: String::new(),
            tags: String::new(),
            area: 0,
            created_at: String::new(),
            platform: "bilibili".into(),
            live_id: "live".into(),
            source: String::new(),
            duration: 1.0,
        })
        .await
        .unwrap();
        create_legacy_projects(db.clone(), output.path().to_path_buf()).await;
        assert_eq!(db.get_projects().await.unwrap().len(), 2);
    }
}
