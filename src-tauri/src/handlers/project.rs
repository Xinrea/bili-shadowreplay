//! Commands for materials (素材) and projects (切片).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use chrono::{Local, Utc};
use recorder::danmu::DanmuEntry;
use recorder::platforms::bilibili::profile::Profile;
use serde::Serialize;
use serde_json::json;

use crate::database::project::ProjectRow;
use crate::database::task::TaskRow;
use crate::database::video::VideoRow;
use crate::database::Database;
use crate::ffmpeg::project::{render_project, ProjectRender, RenderClip};
use crate::handlers::subtitle_fonts::prepare_subtitle_font_directory;
use crate::handlers::utils::get_disk_info_inner;
use crate::handlers::video::{
    ass_primary_font_name, auto_subtitle_material, create_material, ensure_material_duration,
    notify_materials_created, run_queued, upload_video_file, validate_ass_document,
};
use crate::material::Peaks;
use crate::progress::progress_reporter::{ProgressReporter, ProgressReporterTrait};
use crate::project::{srt_cues, Timeline};
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
    /// `material` (one untouched material, nothing to export), `none` (never
    /// exported), `stale` (edited after the export) or `ready`.
    pub export_state: &'static str,
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
    /// The queued export, or `None` when nothing needs to be rendered.
    pub task: Option<TaskRow>,
    pub message: String,
}

async fn output_dir(state: &State) -> PathBuf {
    PathBuf::from(&state.config.read().await.output)
}

/// Identifies what an export of `timeline` contains.
async fn export_hash(state: &State, timeline: &Timeline) -> String {
    let extra = if timeline.danmu {
        serde_json::to_string(&state.config.read().await.danmu_ass_options).unwrap_or_default()
    } else {
        String::new()
    };
    timeline.render_hash(&extra)
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

fn export_state(
    project: &ProjectRow,
    timeline: &Timeline,
    hash: &str,
    materials: &HashMap<i64, VideoRow>,
    output: &Path,
) -> &'static str {
    let single = timeline
        .clips
        .first()
        .and_then(|clip| materials.get(&clip.material_id))
        .map(material_duration)
        .and_then(|duration| timeline.single_full_material(duration));
    if single.is_some() {
        "material"
    } else if project.output.is_empty() {
        "none"
    } else if project.output_hash == hash && output.join(&project.output).exists() {
        "ready"
    } else {
        "stale"
    }
}

async fn project_view(
    state: &State,
    project: ProjectRow,
    materials: &HashMap<i64, VideoRow>,
) -> ProjectView {
    let timeline = Timeline::parse(&project.timeline).unwrap_or_default();
    let hash = export_hash(state, &timeline).await;
    let output = output_dir(state).await;
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
        export_state: export_state(&project, &timeline, &hash, materials, &output),
        duration: timeline.duration(),
        clip_count: timeline.clips.len(),
        display_cover,
        project,
    }
}

async fn single_project_view(state: &State, project: ProjectRow) -> Result<ProjectView, String> {
    let timeline = Timeline::parse(&project.timeline).unwrap_or_default();
    let materials = materials_by_id(state, &timeline.material_ids()).await?;
    Ok(project_view(state, project, &materials).await)
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
        views.push(project_view(&state, project, &materials).await);
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

async fn write_project_cover(
    output: &Path,
    project_id: i64,
    cover: &str,
) -> Result<String, String> {
    let base64 = cover
        .split("base64,")
        .nth(1)
        .ok_or_else(|| "Cover is not a base64 data URL".to_string())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64)
        .map_err(|e| format!("Failed to decode cover: {e}"))?;
    let relative = format!("{}/cover.jpg", project_dir(project_id));
    let path = output.join(&relative);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|e| e.to_string())?;
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
        if first.title.is_empty() {
            Path::new(&first.file)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string()
        } else {
            first.title.clone()
        }
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
            output: String::new(),
            output_hash: String::new(),
            status: 0,
            bvid: String::new(),
            created_at: now.clone(),
            updated_at: now,
        })
        .await?;
    if let Some(cover) = new.cover.filter(|cover| !cover.is_empty()) {
        match write_project_cover(&output, project.id, cover).await {
            Ok(relative) => {
                project.cover = relative;
                state.db.update_project(&project).await?;
            }
            Err(e) => log::warn!("Failed to save project cover: {e}"),
        }
    }
    Ok(project_view(state, project, &materials).await)
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
    let mut project = state.db.get_project(id).await?;
    project.title = title;
    project.note = note;
    project.updated_at = Local::now().to_rfc3339();
    Ok(state.db.update_project(&project).await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_project_timeline(
    state: state_type!(),
    id: i64,
    timeline: Timeline,
) -> Result<ProjectView, String> {
    let mut project = state.db.get_project(id).await?;
    project.timeline = timeline.to_json();
    project.updated_at = Local::now().to_rfc3339();
    state.db.update_project(&project).await?;
    single_project_view(&state, project).await
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_project_profile(
    state: state_type!(),
    id: i64,
    profile: serde_json::Value,
) -> Result<(), String> {
    let mut project = state.db.get_project(id).await?;
    project.profile = profile.to_string();
    Ok(state.db.update_project(&project).await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_project_cover(
    state: state_type!(),
    id: i64,
    cover: String,
) -> Result<String, String> {
    let mut project = state.db.get_project(id).await?;
    project.cover = write_project_cover(&output_dir(&state).await, id, &cover).await?;
    state.db.update_project(&project).await?;
    Ok(project.cover)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn delete_project(state: state_type!(), id: i64) -> Result<(), String> {
    state.db.delete_project(id).await?;
    let directory = output_dir(&state).await.join(project_dir(id));
    if directory.exists() {
        if let Err(e) = tokio::fs::remove_dir_all(&directory).await {
            log::warn!("删除切片目录失败: {} - {e}", directory.display());
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
    for mut project in projects_using(&state.db, material_id).await? {
        let mut timeline = Timeline::parse(&project.timeline)?;
        timeline.remove_material(material_id);
        project.timeline = timeline.to_json();
        project.updated_at = Local::now().to_rfc3339();
        state.db.update_project(&project).await?;
    }
    Ok(())
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
    let mut project = state.db.get_project(project_id).await?;
    let mut timeline = Timeline::parse(&project.timeline)?;
    let materials = HashMap::from([(material.id, material.clone())]);
    import_material_subtitles(
        &output_dir(state).await,
        &mut timeline,
        &materials,
        Some(material.id),
    )
    .await;
    project.timeline = timeline.to_json();
    Ok(state.db.update_project(&project).await?)
}

/// The file to upload: the export, or the material itself when the project is
/// one untouched material.
async fn upload_source(
    state: &State,
    project: &ProjectRow,
    timeline: &Timeline,
) -> Result<PathBuf, String> {
    let output = output_dir(state).await;
    let materials = materials_by_id(state, &timeline.material_ids()).await?;
    let hash = export_hash(state, timeline).await;
    match export_state(project, timeline, &hash, &materials, &output) {
        "material" => {
            let material = &materials[&timeline.clips[0].material_id];
            Ok(output.join(&material.file))
        }
        "ready" => Ok(output.join(&project.output)),
        "stale" => Err("切片在导出后又被修改过，请重新导出后再投稿".to_string()),
        _ => Err("切片还没有导出，请先导出".to_string()),
    }
}

async fn upload_cover(
    state: &State,
    project: &ProjectRow,
    timeline: &Timeline,
) -> Result<PathBuf, String> {
    let output = output_dir(state).await;
    if !project.cover.is_empty() {
        return Ok(output.join(&project.cover));
    }
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

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn upload_project(
    state: state_type!(),
    event_id: String,
    uid: String,
    id: i64,
    profile: Profile,
) -> Result<String, String> {
    let reporter = ProgressReporter::new(state.db.clone(), &state.events, &event_id).await?;
    let mut project = state.db.get_project(id).await?;
    state
        .db
        .add_task(&TaskRow {
            id: event_id.clone(),
            task_type: "upload_project".to_string(),
            status: "pending".to_string(),
            message: String::new(),
            metadata: json!({
                "uid": uid,
                "project_id": id,
                "profile": { "title": profile.title },
            })
            .to_string(),
            created_at: Utc::now().to_rfc3339(),
        })
        .await?;
    let result = async {
        let timeline = Timeline::parse(&project.timeline)?;
        let file = upload_source(&state, &project, &timeline).await?;
        let cover = upload_cover(&state, &project, &timeline).await?;
        upload_video_file(
            &state,
            &reporter,
            &uid,
            &project.room_id,
            &file,
            &cover,
            profile,
        )
        .await
    }
    .await;
    match result {
        Ok(posted) => {
            project.status = 1;
            project.bvid = posted.bvid.clone();
            project.profile = serde_json::to_string(&posted.profile).unwrap_or_default();
            state.db.update_project(&project).await?;
            state
                .db
                .update_task(&event_id, "success", "投稿成功", None)
                .await?;
            Ok(posted.bvid)
        }
        Err(e) => {
            reporter.finish(false, &format!("投稿失败: {e}")).await;
            state
                .db
                .update_task(&event_id, "failed", &format!("投稿失败: {e}"), None)
                .await?;
            Err(e)
        }
    }
}

/// Everything an export needs, fixed when it is queued so that later edits
/// cannot change a waiting job.
struct ExportJob {
    project_id: i64,
    timeline: Timeline,
    hash: String,
    clips: Vec<(PathBuf, f64, f64)>,
    subtitle_ass: Option<String>,
    font_name: Option<String>,
    output: PathBuf,
    cache: PathBuf,
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn export_project(
    state: state_type!(),
    event_id: String,
    id: i64,
    timeline: Timeline,
    subtitle_ass: Option<String>,
) -> Result<ExportStarted, String> {
    let mut project = state.db.get_project(id).await?;
    let materials = materials_by_id(&state, &timeline.material_ids()).await?;
    timeline.validate(|id| materials.get(&id).map(material_duration))?;
    project.timeline = timeline.to_json();
    project.updated_at = Local::now().to_rfc3339();
    state.db.update_project(&project).await?;

    let output = output_dir(&state).await;
    let hash = export_hash(&state, &timeline).await;
    match export_state(&project, &timeline, &hash, &materials, &output) {
        "material" => {
            return Ok(ExportStarted {
                task: None,
                message: "切片只包含一个完整素材，投稿时直接使用素材文件，无需导出".to_string(),
            })
        }
        "ready" => {
            return Ok(ExportStarted {
                task: None,
                message: "导出文件已是最新".to_string(),
            })
        }
        _ => {}
    }

    let subtitle_ass = subtitle_ass.filter(|_| timeline.has_visible_subtitles());
    if let Some(ass) = &subtitle_ass {
        validate_ass_document(ass)?;
    }
    let font_name = subtitle_ass
        .as_deref()
        .map(ass_primary_font_name)
        .transpose()?;
    check_free_space(&state).await?;

    let job = ExportJob {
        project_id: id,
        clips: timeline
            .clips
            .iter()
            .map(|clip| {
                (
                    output.join(&materials[&clip.material_id].file),
                    clip.start,
                    clip.duration(),
                )
            })
            .collect(),
        timeline,
        hash,
        subtitle_ass,
        font_name,
        output,
        cache: PathBuf::from(&state.config.read().await.cache),
    };
    let task = TaskRow {
        id: event_id.clone(),
        task_type: "export_project".to_string(),
        status: "pending".to_string(),
        message: "等待导出".to_string(),
        metadata: json!({ "project_id": id, "title": project.title }).to_string(),
        created_at: Utc::now().to_rfc3339(),
    };
    state.db.add_task(&task).await?;
    let reporter = ProgressReporter::new(state.db.clone(), &state.events, &event_id).await?;

    #[cfg(feature = "gui")]
    let state_clone = (*state).clone();
    #[cfg(feature = "headless")]
    let state_clone = state.clone();
    let task_id = event_id.clone();
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
                result
            },
        ))
        .await;
    if let Err(error) = enqueue_result {
        state
            .db
            .update_task(
                &event_id,
                "failed",
                &format!("加入导出队列失败: {error}"),
                None,
            )
            .await?;
        return Err(error);
    }
    Ok(ExportStarted {
        task: Some(task),
        message: "已加入导出队列".to_string(),
    })
}

async fn export_project_inner(
    state: &State,
    reporter: &ProgressReporter,
    job: ExportJob,
) -> Result<(), String> {
    let mut clips = Vec::with_capacity(job.clips.len());
    for (path, start, duration) in &job.clips {
        let metadata = ffmpeg_utils::extract_video_metadata(path).await?;
        clips.push(RenderClip {
            path: path.clone(),
            start: *start,
            duration: *duration,
            has_audio: !metadata.audio_codec.is_empty(),
            fps: metadata.fps,
        });
    }

    let danmus = if job.timeline.danmu {
        let mut by_material = HashMap::new();
        for clip in &job.timeline.clips {
            if by_material.contains_key(&clip.material_id) {
                continue;
            }
            if let Ok(material) = state.db.get_video(clip.material_id).await {
                let danmus = crate::material::load_danmus(&job.output.join(&material.file)).await;
                by_material.insert(clip.material_id, danmus);
            }
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

    let relative = format!("{}/export_{}.mp4", project_dir(job.project_id), job.hash);
    let output_path = job.output.join(&relative);
    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    render_project(
        reporter,
        ProjectRender {
            clips,
            subtitle_ass: job.subtitle_ass,
            fonts_directory: fonts_directory.as_ref().map(|directory| directory.path()),
            danmus,
            danmu_options: state.config.read().await.danmu_ass_options.clone(),
            output: &output_path,
        },
    )
    .await?;

    // The project may have been edited while this export ran; the hash tells.
    let mut project = state.db.get_project(job.project_id).await?;
    let previous = std::mem::replace(&mut project.output, relative.clone());
    project.output_hash = job.hash;
    state.db.update_project(&project).await?;
    if !previous.is_empty() && previous != relative {
        let _ = tokio::fs::remove_file(job.output.join(previous)).await;
    }
    state
        .db
        .new_message("切片导出完成", &format!("切片「{}」已导出", project.title))
        .await?;
    Ok(())
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
        let title = if video.title.is_empty() {
            Path::new(&video.file)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string()
        } else {
            video.title.clone()
        };
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
            output: String::new(),
            output_hash: String::new(),
            status: i64::from(posted),
            bvid: video.bvid.clone(),
            created_at: video.created_at.clone(),
            updated_at: video.created_at.clone(),
        };
        if let Err(e) = db.add_project(&project).await {
            log::error!("Failed to create project for video {}: {e}", video.id);
            continue;
        }
        if let Err(e) = db.clear_video_needs_project(video.id).await {
            log::error!("Failed to mark video {} as migrated: {e}", video.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::credentials::CredentialCipher;

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
