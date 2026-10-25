//! BSR operations exposed to external MCP clients. Shared schemas and data
//! readers keep this catalog aligned with the in-app assistant; writes use the
//! same handlers as the application UI after the user enables the MCP server.

use base64::Engine;
use recorder::platforms::PlatformType;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Component, Path};

#[cfg(feature = "gui")]
use tauri::Manager;

use super::{jobs, workflow};
use crate::{agent, handlers, state::State};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Read,
    RemoveAccount,
    AddRecorder,
    RemoveRecorder,
    DeleteArchive,
    DeleteArchives,
    DeleteBackgroundTask,
    GetVideoCover,
    GetVideoSubtitle,
    DeleteVideo,
    GenerateVideoSubtitle,
    EncodeVideoSubtitle,
    ClipRange,
    GenerateArchiveSubtitle,
    ExtractVideoFrames,
    #[cfg(feature = "gui")]
    GetVideoMetadata,
    #[cfg(feature = "gui")]
    GetArchiveMetadata,
    #[cfg(feature = "gui")]
    MergeVideos,
    #[cfg(feature = "gui")]
    ExtractVideoAudio,
    GetTask,
    CancelTask,
    GenerateMaterials,
    GetProjects,
    GetProject,
    CreateProject,
    SetProjectClips,
    UpdateProjectInfo,
    UpdateProjectOutputSettings,
    DeleteProject,
    GetVideoTypelist,
    GetUploads,
    GetUpload,
    CreateUploadDraft,
    UpdateUploadDraft,
    UpdateUploadDanmu,
    DeleteUpload,
}

impl Action {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "get_accounts"
            | "get_recorder_list"
            | "get_recorder_info"
            | "get_archives"
            | "get_archive"
            | "get_recent_record"
            | "get_recent_record_all"
            | "get_videos"
            | "get_all_videos"
            | "get_video"
            | "get_background_tasks"
            | "get_archive_subtitle"
            | "get_danmu_record"
            | "analyze_danmu_highlights"
            | "search_danmu_keywords" => Self::Read,
            "remove_account" => Self::RemoveAccount,
            "add_recorder" => Self::AddRecorder,
            "remove_recorder" => Self::RemoveRecorder,
            "delete_archive" => Self::DeleteArchive,
            "delete_archives" => Self::DeleteArchives,
            "delete_background_task" => Self::DeleteBackgroundTask,
            "get_video_cover" => Self::GetVideoCover,
            "get_video_subtitle" => Self::GetVideoSubtitle,
            "delete_video" => Self::DeleteVideo,
            "generate_video_subtitle" => Self::GenerateVideoSubtitle,
            "encode_video_subtitle" => Self::EncodeVideoSubtitle,
            "clip_range" => Self::ClipRange,
            "generate_archive_subtitle" => Self::GenerateArchiveSubtitle,
            "extract_video_frames" => Self::ExtractVideoFrames,
            #[cfg(feature = "gui")]
            "get_video_metadata" => Self::GetVideoMetadata,
            #[cfg(feature = "gui")]
            "get_archive_metadata" => Self::GetArchiveMetadata,
            #[cfg(feature = "gui")]
            "merge_videos" => Self::MergeVideos,
            #[cfg(feature = "gui")]
            "extract_video_audio" => Self::ExtractVideoAudio,
            "get_task" => Self::GetTask,
            "cancel_task" => Self::CancelTask,
            "generate_materials" => Self::GenerateMaterials,
            "get_projects" => Self::GetProjects,
            "get_project" => Self::GetProject,
            "create_project" => Self::CreateProject,
            "set_project_clips" => Self::SetProjectClips,
            "update_project_info" => Self::UpdateProjectInfo,
            "update_project_output_settings" => Self::UpdateProjectOutputSettings,
            "delete_project" => Self::DeleteProject,
            "get_video_typelist" => Self::GetVideoTypelist,
            "get_uploads" => Self::GetUploads,
            "get_upload" => Self::GetUpload,
            "create_upload_draft" => Self::CreateUploadDraft,
            "update_upload_draft" => Self::UpdateUploadDraft,
            "update_upload_danmu" => Self::UpdateUploadDanmu,
            "delete_upload" => Self::DeleteUpload,
            _ => return None,
        })
    }

    fn readonly(self) -> bool {
        match self {
            Self::Read
            | Self::GetVideoCover
            | Self::GetVideoSubtitle
            | Self::ExtractVideoFrames
            | Self::GetTask
            | Self::GetProjects
            | Self::GetProject
            | Self::GetVideoTypelist
            | Self::GetUploads
            | Self::GetUpload => true,
            #[cfg(feature = "gui")]
            Self::GetVideoMetadata | Self::GetArchiveMetadata => true,
            _ => false,
        }
    }

    fn destructive(self) -> bool {
        matches!(
            self,
            Self::RemoveAccount
                | Self::RemoveRecorder
                | Self::DeleteArchive
                | Self::DeleteArchives
                | Self::DeleteBackgroundTask
                | Self::DeleteVideo
                | Self::GenerateVideoSubtitle
                | Self::GenerateArchiveSubtitle
                | Self::SetProjectClips
                | Self::DeleteProject
                | Self::DeleteUpload
        )
    }
}

fn resolve_action(name: &str, readonly: bool) -> Result<Action, String> {
    let action = Action::from_name(name).ok_or_else(|| format!("Unknown BSR tool: {name}"))?;
    if readonly && !action.readonly() {
        return Err("BSR is running in read-only mode; this tool is disabled".into());
    }
    Ok(action)
}

pub(super) fn definitions(readonly: bool) -> Vec<Value> {
    let shared = agent::bsr_tool_definitions();
    let clip_range_params = shared
        .iter()
        .find(|schema| schema["properties"]["action"]["const"] == "clip_range")
        .map(|schema| schema["properties"]["args"]["properties"]["clip_range_params"].clone())
        .unwrap_or_default();
    let own = workflow::definitions(clip_range_params);
    let own_names: Vec<&str> = own.iter().map(|(name, _, _)| *name).collect();
    shared
        .into_iter()
        .filter_map(|schema| {
            let name = schema["properties"]["action"]["const"].as_str()?.to_string();
            // Tools defined for MCP replace the assistant's of the same name.
            if own_names.contains(&name.as_str()) {
                return None;
            }
            // Confirmation is provided by external clients. The in-app
            // assistant retains its own, separate confirmation behavior.
            let mut description = schema["description"]
                .as_str()
                .unwrap_or_default()
                .replace(" after user confirmation because the result is handled locally", "")
                .replace(" after user confirmation", "");
            match name.as_str() {
                "get_video_cover" => description = "Get a video's cover as an image, with its ID and stored cover filename as metadata.".into(),
                "clip_range" => description = "Cut one material joining one or more archive ranges, in the background. Returns a task_id; poll get_task, whose metadata.result has video_id once it succeeds. generate_materials cuts one material per range instead.".into(),
                "generate_video_subtitle" => description = "Recognize and save a video's subtitles, overwriting existing ones, in the background. Returns a task_id; read them with get_video_subtitle once get_task reports success.".into(),
                "generate_archive_subtitle" => description = "Recognize and save an archive's subtitles, overwriting existing ones, in the background. Returns a task_id; read them with get_archive_subtitle once get_task reports success.".into(),
                _ => {}
            }
            Some((name, description, schema["properties"]["args"].clone()))
        })
        .chain(
            own.into_iter()
                .map(|(name, description, schema)| (name.to_string(), description, schema)),
        )
        .filter_map(|(name, description, mut input_schema)| {
            let action = resolve_action(&name, readonly).ok()?;
            include_supported_platforms(&mut input_schema);
            constrain_schema(&name, &mut input_schema);
            Some(json!({
                "name": name,
                "description": description,
                "inputSchema": input_schema,
                "annotations": {
                    "readOnlyHint": action.readonly(),
                    "destructiveHint": action.destructive(),
                    "idempotentHint": action.readonly(),
                    "openWorldHint": matches!(action, Action::AddRecorder | Action::RemoveAccount | Action::GenerateArchiveSubtitle | Action::GenerateVideoSubtitle | Action::GetVideoTypelist)
                }
            }))
        })
        .collect()
}

fn include_supported_platforms(schema: &mut Value) {
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        for (name, property) in properties {
            if name == "platform" {
                property["enum"] = json!(PlatformType::ALL.map(|platform| platform.as_str()));
            }
            include_supported_platforms(property);
        }
    }
    if let Some(items) = schema.get_mut("items") {
        include_supported_platforms(items);
    }
}

fn constrain_schema(name: &str, schema: &mut Value) {
    let properties = &mut schema["properties"];
    match name {
        "remove_account" => {
            // Account IDs are opaque strings on several supported platforms.
            properties["uid"]["type"] = json!("string");
        }
        "get_archives" | "get_recent_record" | "get_recent_record_all" => {
            properties["offset"]["minimum"] = json!(0);
            properties["limit"]["minimum"] = json!(1);
            properties["limit"]["maximum"] = json!(1000);
        }
        "analyze_danmu_highlights" => {
            properties["time_window"]["minimum"] = json!(1);
            properties["min_density"]["minimum"] = json!(1);
        }
        "search_danmu_keywords" => {
            properties["context_seconds"]["minimum"] = json!(0);
            properties["keywords"]["minItems"] = json!(1);
            properties["keywords"]["maxItems"] = json!(100);
            properties["keywords"]["items"]["minLength"] = json!(1);
        }
        "delete_archives" => {
            properties["live_ids"]["minItems"] = json!(1);
        }
        "encode_video_subtitle" => {
            properties["srt_style"]["description"] = json!("Comma-separated ASS overrides using FontName, FontSize, PrimaryColour, OutlineColour, Outline, Alignment, MarginV, MarginL, MarginR. Font names may contain letters, digits, spaces, hyphens, underscores, periods or parentheses. Colors use &H followed by 6 or 8 hex digits. Use an empty string for default styling; filter expressions, paths and URLs are not accepted.");
            properties["srt_style"]["maxLength"] = json!(2048);
        }
        "clip_range" => {
            if let Some(properties) = properties.as_object_mut() {
                properties.remove("reason");
            }
            constrain_clip_range_params(&mut properties["clip_range_params"]);
            if let Some(required) = schema["required"].as_array_mut() {
                required.retain(|name| name != "reason");
            }
        }
        "generate_materials" => {
            constrain_clip_range_params(&mut properties["clip_range_params"]);
            // Each range is its own material unless they are merged.
            properties["clip_range_params"]["properties"]["transition"]["description"] =
                json!("Transition between ranges joined with merge.");
        }
        "get_danmu_record" => {
            properties["start"]["minimum"] = json!(0);
            properties["end"]["exclusiveMinimum"] = json!(0);
        }
        "extract_video_frames" => {
            properties["max_frames"]["minimum"] = json!(1);
            properties["max_frames"]["maximum"] = json!(10);
            properties["max_frames"]["default"] = json!(5);
            properties["timestamps"]["maxItems"] = json!(10);
            properties["timestamps"]["items"]["minimum"] = json!(0);
        }
        "merge_videos" => {
            properties["video_ids"]["minItems"] = json!(1);
        }
        _ => {}
    }
}

/// Fields of `clip_range_params` clients may leave out.
const CLIP_RANGE_DEFAULTS: [&str; 5] = ["note", "cover", "danmu", "local_offset", "fix_encoding"];

/// The value of a field of `clip_range_params` left out.
fn clip_range_default(field: &str) -> Value {
    match field {
        "danmu" | "fix_encoding" => json!(false),
        "local_offset" => json!(0),
        _ => json!(""),
    }
}

/// Describe the clip parameters for clients, which may omit the fields with
/// defaults.
fn constrain_clip_range_params(params: &mut Value) {
    let clip = &mut params["properties"];
    // The backend stores this offset as an i64, although the older
    // assistant schema allowed arbitrary JSON numbers.
    clip["local_offset"]["type"] = json!("integer");
    clip["local_offset"]["description"] = json!("Seconds the danmaku were realigned by in the player; 0 unless the user adjusted it. Default 0.");
    clip["ranges"]["minItems"] = json!(1);
    clip["ranges"]["description"] = json!("Seconds from the start of the recording, the time base of get_danmu_record, analyze_danmu_highlights and search_danmu_keywords.");
    clip["ranges"]["items"]["properties"]["start"]["minimum"] = json!(0);
    clip["ranges"]["items"]["properties"]["end"]["exclusiveMinimum"] = json!(0);
    clip["danmu"]["description"] = json!("Burn the danmaku into the material. By default they are kept beside it, for projects and uploads to burn later.");
    clip["cover"]["description"] =
        json!("Cover as a base64 image data URL; empty to capture a frame. Default empty.");
    clip["fix_encoding"]["description"] =
        json!("Re-encode the material to repair broken timestamps; slower. Default false.");
    clip["transition"]["description"] = json!("Transition between joined ranges.");
    if let Some(required) = params["required"].as_array_mut() {
        required.retain(|name| !CLIP_RANGE_DEFAULTS.iter().any(|field| name == *field));
    }
}

/// The clip parameters in `args`, with defaults for the fields left out.
fn clip_range_params(args: &Value) -> Result<crate::recorder_manager::ClipRangeParams, String> {
    let mut params = args
        .get("clip_range_params")
        .cloned()
        .ok_or_else(|| "Missing argument: clip_range_params".to_string())?;
    if let Some(fields) = params.as_object_mut() {
        for field in CLIP_RANGE_DEFAULTS {
            fields
                .entry(field)
                .or_insert_with(|| clip_range_default(field));
        }
    }
    serde_json::from_value(params)
        .map_err(|error| format!("Invalid argument clip_range_params: {error}"))
}

pub(super) fn argument<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T, String> {
    let value = args
        .get(name)
        .ok_or_else(|| format!("Missing argument: {name}"))?;
    serde_json::from_value(value.clone())
        .map_err(|error| format!("Invalid argument {name}: {error}"))
}

/// The backend interpolates force_style inside a quoted FFmpeg filter. Accept
/// only the override fields produced by SubtitleStyleEditor, with value
/// grammars that cannot close the quote or introduce another filter/option.
fn validate_subtitle_style(style: &str) -> Result<(), String> {
    if style.len() > 2048 || style.chars().any(char::is_control) {
        return Err(
            "srt_style must be at most 2048 bytes and contain no control characters".into(),
        );
    }
    if style.trim().is_empty() {
        return Ok(());
    }

    let mut fields = HashSet::new();
    for override_value in style.split(',') {
        let (field, value) = override_value.split_once('=').ok_or_else(|| {
            "srt_style must contain comma-separated Key=Value overrides".to_string()
        })?;
        let field = field.trim();
        let value = value.trim();
        if !fields.insert(field) {
            return Err(format!("Duplicate subtitle style field: {field}"));
        }
        let valid = match field {
            "FontName" => {
                !value.is_empty()
                    && value.chars().all(|character| {
                        character.is_alphanumeric()
                            || matches!(character, ' ' | '-' | '_' | '.' | '(' | ')')
                    })
            }
            "PrimaryColour" | "OutlineColour" => {
                let color = value
                    .strip_prefix("&H")
                    .or_else(|| value.strip_prefix("&h"))
                    .unwrap_or_default();
                let color = color.strip_suffix('&').unwrap_or(color);
                matches!(color.len(), 6 | 8)
                    && color.bytes().all(|character| character.is_ascii_hexdigit())
            }
            "FontSize" | "Outline" => {
                value
                    .bytes()
                    .all(|character| character.is_ascii_digit() || character == b'.')
                    && value.parse::<f64>().is_ok_and(|number| {
                        number.is_finite()
                            && if field == "FontSize" {
                                number > 0.0
                            } else {
                                (0.0..=4.0).contains(&number)
                            }
                    })
            }
            "Alignment" => {
                value.bytes().all(|character| character.is_ascii_digit())
                    && value
                        .parse::<u8>()
                        .is_ok_and(|number| (1..=11).contains(&number))
            }
            "MarginV" | "MarginL" | "MarginR" => {
                value.bytes().all(|character| character.is_ascii_digit())
                    && value.parse::<i32>().is_ok_and(|number| number >= 0)
            }
            _ => return Err(format!("Unsupported subtitle style field: {field}")),
        };
        if !valid {
            return Err(format!("Invalid subtitle style value for {field}"));
        }
    }
    Ok(())
}

/// Validate semantic constraints at the execution boundary as well as in the
/// advertised schema, so direct calls cannot bypass read-only and range limits.
fn validate_arguments(name: &str, args: &Value) -> Result<(), String> {
    if !args.is_object() {
        return Err("Tool arguments must be an object".into());
    }
    match name {
        "get_archives" | "get_recent_record" | "get_recent_record_all" => {
            let offset: i64 = argument(args, "offset")?;
            let limit: i64 = argument(args, "limit")?;
            if offset < 0 || !(1..=1000).contains(&limit) {
                return Err(
                    "offset must be non-negative and limit must be between 1 and 1000".into(),
                );
            }
        }
        "analyze_danmu_highlights" => {
            let window: f64 = argument(args, "time_window")?;
            let density: i64 = argument(args, "min_density")?;
            if !window.is_finite() || window < 1.0 || density < 1 {
                return Err(
                    "time_window must be at least 1 second and min_density must be positive".into(),
                );
            }
        }
        "search_danmu_keywords" => {
            let context: f64 = argument(args, "context_seconds")?;
            let keywords: Vec<String> = argument(args, "keywords")?;
            if !context.is_finite() || context < 0.0 {
                return Err("context_seconds must be non-negative".into());
            }
            if keywords.is_empty() || keywords.len() > 100 || keywords.iter().any(String::is_empty)
            {
                return Err("keywords must contain between 1 and 100 non-empty strings".into());
            }
        }
        "delete_archives" => {
            let ids: Vec<String> = argument(args, "live_ids")?;
            if ids.is_empty() {
                return Err("live_ids must not be empty".into());
            }
        }
        "encode_video_subtitle" => {
            let style: String = argument(args, "srt_style")?;
            validate_subtitle_style(&style)?;
        }
        "get_danmu_record" => {
            let start: Option<f64> = args
                .get("start")
                .map(|_| argument(args, "start"))
                .transpose()?;
            let end: Option<f64> = args.get("end").map(|_| argument(args, "end")).transpose()?;
            if start.is_some_and(|start| !start.is_finite() || start < 0.0)
                || end.is_some_and(|end| !end.is_finite() || end <= start.unwrap_or(0.0))
            {
                return Err("start must be non-negative and end must be after start".into());
            }
        }
        "set_project_clips" => {
            let clips: Vec<crate::handlers::project::ClipRequest> = argument(args, "clips")?;
            if clips.is_empty()
                || clips.iter().any(|clip| {
                    !clip.start.is_finite()
                        || clip.start < 0.0
                        || clip
                            .end
                            .is_some_and(|end| !end.is_finite() || end <= clip.start)
                })
            {
                return Err(
                    "Each clip must have 0 <= start < end; provide at least one clip".into(),
                );
            }
        }
        "clip_range" | "generate_materials" => {
            let params = clip_range_params(args)?;
            if params.ranges.is_empty()
                || params.ranges.iter().any(|range| {
                    !range.start.is_finite()
                        || !range.end.is_finite()
                        || range.start < 0.0
                        || range.end <= range.start
                })
            {
                return Err(
                    "Each clip range must have 0 <= start < end; provide at least one range".into(),
                );
            }
        }
        "extract_video_frames" => {
            let max_frames: usize = args
                .get("max_frames")
                .map(|_| argument(args, "max_frames"))
                .transpose()?
                .unwrap_or(5);
            let timestamps: Vec<f64> = args
                .get("timestamps")
                .map(|_| argument(args, "timestamps"))
                .transpose()?
                .unwrap_or_default();
            if !(1..=10).contains(&max_frames)
                || timestamps.len() > 10
                || timestamps
                    .iter()
                    .any(|time| !time.is_finite() || *time < 0.0)
            {
                return Err("max_frames must be between 1 and 10; timestamps must contain at most 10 non-negative values".into());
            }
        }
        "merge_videos" => {
            let ids: Vec<i64> = argument(args, "video_ids")?;
            if ids.is_empty() {
                return Err("video_ids must not be empty".into());
            }
        }
        _ => {}
    }
    Ok(())
}

/// Read current file-backed covers and legacy inline covers without duplicating
/// the image bytes in the text/structured metadata.
async fn video_cover_result(output_dir: &Path, id: i64, cover: &str) -> Result<Value, String> {
    if cover.is_empty() {
        return Err(format!("Video {id} has no cover"));
    }
    let inline = cover.starts_with("data:");
    let bytes = if inline {
        let (_, data) = cover
            .split_once(',')
            .filter(|(header, _)| header.starts_with("data:image/") && header.ends_with(";base64"))
            .ok_or_else(|| format!("Video {id} cover must be a Base64 image data URL"))?;
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|error| format!("Failed to decode video {id} cover: {error}"))?
    } else {
        if !Path::new(cover)
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
        {
            return Err(format!(
                "Video {id} cover must be a relative path inside the output directory"
            ));
        }
        let cover_path = output_dir.join(cover);
        tokio::fs::read(&cover_path).await.map_err(|error| {
            format!(
                "Failed to read video cover {}: {error}",
                cover_path.display()
            )
        })?
    };
    if bytes.is_empty() {
        return Err(format!("Video cover is empty for video {id}"));
    }
    // Cover updates write to a .jpg filename even when the supplied data URL
    // contains another image format. Determine the MIME type from the bytes.
    let mime_type = if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        return Err(format!(
            "Unsupported video cover image format for video {id}"
        ));
    };
    Ok(json!({
        "response": { "id": id, "cover": if inline { None } else { Some(cover) } },
        "parts": [{
            "type": "image",
            "data": base64::engine::general_purpose::STANDARD.encode(bytes),
            "mimeType": mime_type
        }]
    }))
}

pub(super) async fn call(state: &State, name: &str, args: Value) -> Result<Value, String> {
    #[cfg(feature = "headless")]
    let readonly = state.readonly;
    #[cfg(not(feature = "headless"))]
    let readonly = false;
    let action = resolve_action(name, readonly)?;
    validate_arguments(name, &args)?;
    if action == Action::Read {
        return agent::execute_bsr_read_tool(state, name, args).await;
    }

    #[cfg(feature = "gui")]
    let command_state = state.app_handle.state::<State>();
    #[cfg(feature = "headless")]
    let command_state = state.clone();

    match action {
        Action::Read => unreachable!("read operations return before handler dispatch"),
        Action::RemoveAccount => {
            handlers::account::remove_account(
                command_state,
                argument(&args, "platform")?,
                argument(&args, "uid")?,
            )
            .await?;
            Ok(json!({ "success": true }))
        }
        Action::AddRecorder => {
            let recorder = handlers::recorder::add_recorder(
                command_state,
                argument(&args, "platform")?,
                argument(&args, "room_id")?,
                argument(&args, "extra")?,
            )
            .await?;
            Ok(json!({ "recorder": recorder }))
        }
        Action::RemoveRecorder => {
            handlers::recorder::remove_recorder(
                command_state,
                argument(&args, "platform")?,
                argument(&args, "room_id")?,
            )
            .await?;
            Ok(json!({ "success": true }))
        }
        Action::DeleteArchive => {
            handlers::recorder::delete_archive(
                command_state,
                argument(&args, "platform")?,
                argument(&args, "room_id")?,
                argument(&args, "live_id")?,
            )
            .await?;
            Ok(json!({ "success": true }))
        }
        Action::DeleteArchives => {
            handlers::recorder::delete_archives(
                command_state,
                argument(&args, "platform")?,
                argument(&args, "room_id")?,
                argument(&args, "live_ids")?,
            )
            .await?;
            Ok(json!({ "success": true }))
        }
        Action::DeleteBackgroundTask => {
            let id: String = argument(&args, "id")?;
            handlers::task::delete_task(command_state, &id).await?;
            Ok(json!({ "success": true }))
        }
        Action::GetVideoCover => {
            let id = argument(&args, "id")?;
            let cover = state
                .db
                .get_video_cover(id)
                .await
                .map_err(|error| error.to_string())?;
            let output_dir = state.config.read().await.output.clone();
            video_cover_result(Path::new(&output_dir), id, &cover).await
        }
        Action::GetVideoSubtitle => {
            let subtitle =
                handlers::video::get_video_subtitle(command_state, argument(&args, "id")?).await?;
            Ok(json!({ "subtitle": subtitle }))
        }
        Action::DeleteVideo => {
            handlers::video::delete_video(command_state, argument(&args, "id")?).await?;
            Ok(json!({ "success": true }))
        }
        Action::GenerateVideoSubtitle => {
            let id: i64 = argument(&args, "id")?;
            let task_id = jobs::new_task_id();
            let background = state.clone();
            let event_id = task_id.clone();
            jobs::start(
                state,
                task_id,
                "generate_video_subtitle",
                None,
                async move {
                    handlers::video::generate_video_subtitle(
                        command_state!(background),
                        event_id,
                        id,
                        None,
                    )
                    .await?;
                    Ok(json!({ "video_id": id }))
                },
            )
            .await
        }
        Action::EncodeVideoSubtitle => {
            let event_id = jobs::new_task_id();
            let task = handlers::video::encode_video_subtitle(
                command_state,
                event_id.clone(),
                argument(&args, "id")?,
                argument(&args, "srt_style")?,
                None,
            )
            .await?;
            Ok(json!({ "task": task, "task_id": event_id, "event_id": event_id }))
        }
        Action::ClipRange => {
            let params = clip_range_params(&args)?;
            let task_id = jobs::new_task_id();
            let background = state.clone();
            let event_id = task_id.clone();
            jobs::start(state, task_id, "clip_range", None, async move {
                let video =
                    handlers::video::clip_range(command_state!(background), event_id, params)
                        .await?;
                Ok(json!({ "video_id": video.id }))
            })
            .await
        }
        Action::GenerateMaterials => {
            let params = clip_range_params(&args)?;
            let merge = args.get("merge").and_then(Value::as_bool).unwrap_or(false);
            let create_project = args
                .get("create_project")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let task_id = jobs::new_task_id();
            let background = state.clone();
            let event_id = task_id.clone();
            jobs::start(state, task_id, "generate_materials", None, async move {
                let generated = handlers::project::generate_materials(
                    command_state!(background),
                    event_id,
                    params,
                    create_project,
                    merge,
                )
                .await?;
                Ok(json!({
                    "material_ids": generated.materials.iter().map(|material| material.id).collect::<Vec<_>>(),
                    "project_id": generated.project.map(|project| project.project.id),
                }))
            })
            .await
        }
        Action::GenerateArchiveSubtitle => {
            let platform: String = argument(&args, "platform")?;
            let room_id: String = argument(&args, "room_id")?;
            let live_id: String = argument(&args, "live_id")?;
            let record = jobs::TaskRecord {
                task_type: "generate_archive_subtitle",
                metadata: json!({ "platform": platform, "room_id": room_id, "live_id": live_id }),
            };
            let background = state.clone();
            jobs::start(
                state,
                jobs::new_task_id(),
                "generate_archive_subtitle",
                Some(record),
                async move {
                    handlers::recorder::generate_archive_subtitle(
                        command_state!(background),
                        platform.clone(),
                        room_id.clone(),
                        live_id.clone(),
                    )
                    .await?;
                    Ok(json!({ "platform": platform, "room_id": room_id, "live_id": live_id }))
                },
            )
            .await
        }
        Action::GetTask => jobs::task(state, &argument::<String>(&args, "id")?).await,
        Action::CancelTask => {
            let id: String = argument(&args, "id")?;
            jobs::task(state, &id).await?;
            handlers::video::cancel(command_state, id.clone()).await?;
            jobs::task(state, &id).await
        }
        Action::GetProjects => workflow::get_projects(state, &args).await,
        Action::GetProject => workflow::get_project(state, &args).await,
        Action::CreateProject => workflow::create_project(state, &args).await,
        Action::SetProjectClips => workflow::set_project_clips(state, &args).await,
        Action::UpdateProjectInfo => workflow::update_project_info(state, &args).await,
        Action::UpdateProjectOutputSettings => {
            workflow::update_project_output_settings(state, &args).await
        }
        Action::DeleteProject => {
            let id = argument(&args, "id")?;
            state
                .db
                .get_project(id)
                .await
                .map_err(|_| format!("Project {id} not found"))?;
            handlers::project::delete_project(command_state, id).await?;
            Ok(json!({ "success": true }))
        }
        Action::GetVideoTypelist => {
            let typelist = handlers::video::get_video_typelist(command_state).await?;
            Ok(json!({ "typelist": typelist }))
        }
        Action::GetUploads => workflow::get_uploads(state).await,
        Action::GetUpload => workflow::get_upload(state, &args).await,
        Action::CreateUploadDraft => workflow::create_upload_draft(state, &args).await,
        Action::UpdateUploadDraft => workflow::update_upload_draft(state, &args).await,
        Action::UpdateUploadDanmu => workflow::update_upload_danmu(state, &args).await,
        Action::DeleteUpload => {
            let id = argument(&args, "id")?;
            state
                .db
                .get_upload(id)
                .await
                .map_err(|_| format!("Upload {id} not found"))?;
            handlers::upload::delete_upload(command_state, id).await?;
            Ok(json!({ "success": true }))
        }
        Action::ExtractVideoFrames => {
            let timestamps = args
                .get("timestamps")
                .map(|_| argument(&args, "timestamps"))
                .transpose()?
                .unwrap_or_default();
            let max_frames = args
                .get("max_frames")
                .map(|_| argument(&args, "max_frames"))
                .transpose()?
                .unwrap_or(5);
            let frames = handlers::video_editing::extract_video_frames(
                command_state,
                argument(&args, "video_id")?,
                timestamps,
                max_frames,
            )
            .await?;
            let timestamps = frames
                .iter()
                .map(|frame| frame.timestamp)
                .collect::<Vec<_>>();
            let parts = frames.into_iter().map(|frame| json!({ "type": "image", "data": frame.image_base64, "mimeType": "image/jpeg" })).collect::<Vec<_>>();
            Ok(
                json!({ "response": { "video_id": args["video_id"], "timestamps": timestamps, "frame_count": parts.len() }, "parts": parts }),
            )
        }
        #[cfg(feature = "gui")]
        Action::GetVideoMetadata => {
            let metadata = handlers::video_editing::get_video_metadata(
                command_state,
                argument(&args, "video_id")?,
            )
            .await?;
            serde_json::to_value(metadata).map_err(|error| error.to_string())
        }
        #[cfg(feature = "gui")]
        Action::GetArchiveMetadata => {
            handlers::video_editing::get_archive_metadata(
                command_state,
                argument(&args, "platform")?,
                argument(&args, "room_id")?,
                argument(&args, "live_id")?,
            )
            .await
        }
        #[cfg(feature = "gui")]
        Action::MergeVideos => {
            let transition = args
                .get("transition")
                .map(|_| argument(&args, "transition"))
                .transpose()?;
            let video_id = handlers::video_editing::merge_videos(
                command_state,
                argument(&args, "video_ids")?,
                argument(&args, "output_title")?,
                argument(&args, "output_note")?,
                transition,
            )
            .await?;
            Ok(json!({ "video_id": video_id }))
        }
        #[cfg(feature = "gui")]
        Action::ExtractVideoAudio => {
            let path = handlers::video_editing::extract_video_audio(
                command_state,
                argument(&args, "video_id")?,
            )
            .await?;
            Ok(json!({ "path": path }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[tokio::test]
    async fn video_cover_reads_output_file_and_returns_image_bytes_with_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let bytes = [0xff, 0xd8, 0xff, 0xd9];
        tokio::fs::write(directory.path().join("cover.jpg"), bytes)
            .await
            .unwrap();
        let result = video_cover_result(directory.path(), 42, "cover.jpg")
            .await
            .unwrap();
        assert_eq!(result["response"], json!({"id": 42, "cover": "cover.jpg"}));
        let image = &result["parts"][0];
        assert_eq!(image["type"], "image");
        assert_eq!(image["mimeType"], "image/jpeg");
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(image["data"].as_str().unwrap())
                .unwrap(),
            bytes
        );
    }

    #[tokio::test]
    async fn video_cover_reports_absent_missing_and_empty_files() {
        let directory = tempfile::tempdir().unwrap();
        assert!(video_cover_result(directory.path(), 42, "")
            .await
            .unwrap_err()
            .contains("has no cover"));
        assert!(video_cover_result(directory.path(), 42, "missing.jpg")
            .await
            .unwrap_err()
            .contains("Failed to read video cover"));
        tokio::fs::write(directory.path().join("empty.jpg"), [])
            .await
            .unwrap();
        assert!(video_cover_result(directory.path(), 42, "empty.jpg")
            .await
            .unwrap_err()
            .contains("Video cover is empty"));
    }

    #[tokio::test]
    async fn video_cover_uses_image_bytes_instead_of_filename_for_mime_type() {
        let directory = tempfile::tempdir().unwrap();
        for (bytes, mime) in [
            (b"\x89PNG\r\n\x1a\n".as_slice(), "image/png"),
            (b"GIF89a".as_slice(), "image/gif"),
            (b"RIFF\x00\x00\x00\x00WEBP".as_slice(), "image/webp"),
        ] {
            tokio::fs::write(directory.path().join("cover.jpg"), bytes)
                .await
                .unwrap();
            let result = video_cover_result(directory.path(), 42, "cover.jpg")
                .await
                .unwrap();
            assert_eq!(result["parts"][0]["mimeType"], mime);
        }
        tokio::fs::write(directory.path().join("cover.jpg"), b"not an image")
            .await
            .unwrap();
        assert!(video_cover_result(directory.path(), 42, "cover.jpg")
            .await
            .unwrap_err()
            .contains("Unsupported video cover image format"));
    }

    #[tokio::test]
    async fn legacy_inline_video_covers_return_images_without_duplicate_base64_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let bytes = b"\x89PNG\r\n\x1a\n";
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        // The byte signature, rather than the legacy header, is authoritative.
        let cover = format!("data:image/jpeg;base64,{data}");
        let result = video_cover_result(directory.path(), 42, &cover)
            .await
            .unwrap();
        assert_eq!(result["parts"][0]["data"], data);
        assert_eq!(result["parts"][0]["mimeType"], "image/png");
        assert_eq!(result["response"], json!({"id": 42, "cover": null}));
        assert!(!result["response"].to_string().contains(&data));
        for invalid in [
            "data:image/png,unencoded",
            "data:image/png;base64,invalid!",
            "data:text/plain;base64,aGVsbG8=",
        ] {
            assert!(video_cover_result(directory.path(), 42, invalid)
                .await
                .is_err());
        }
    }

    #[tokio::test]
    async fn video_cover_rejects_absolute_paths_and_parent_traversal() {
        let directory = tempfile::tempdir().unwrap();
        for cover in [
            directory.path().join("cover.jpg").to_str().unwrap(),
            "../cover.jpg",
            "nested/../../cover.jpg",
        ] {
            assert!(video_cover_result(directory.path(), 42, cover)
                .await
                .unwrap_err()
                .contains("must be a relative path"));
        }
    }

    #[test]
    fn catalog_contains_only_unique_executable_tools() {
        let tools = definitions(false);
        let mut names = HashSet::new();
        for tool in &tools {
            let name = tool["name"].as_str().unwrap();
            assert!(names.insert(name));
            assert!(resolve_action(name, false).is_ok());
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        }
        for name in [
            "get_accounts",
            "extract_video_frames",
            "get_video_cover",
            "get_danmu_record",
            "clip_range",
            "add_recorder",
            "delete_video",
        ] {
            assert!(names.contains(name));
        }
        for name in [
            "open_clip",
            "list_folder",
            "generic_ffmpeg_command",
            "post_video_to_bilibili",
        ] {
            assert!(!names.contains(name));
            assert!(resolve_action(name, false).is_err());
        }
    }

    #[test]
    fn readonly_catalog_and_execution_boundary_reject_mutations() {
        let tools = definitions(true);
        assert!(!tools.is_empty());
        assert!(tools
            .iter()
            .all(|tool| tool["annotations"]["readOnlyHint"] == true));
        assert!(resolve_action("get_recorder_list", true).is_ok());
        assert!(resolve_action("get_video_subtitle", true).is_ok());
        assert!(resolve_action("extract_video_frames", true).is_ok());
        assert!(resolve_action("get_video_cover", true).is_ok());
        for name in [
            "add_recorder",
            "delete_archive",
            "clip_range",
            "generate_archive_subtitle",
        ] {
            assert!(resolve_action(name, true).is_err());
            assert!(!tools.iter().any(|tool| tool["name"] == name));
        }
    }

    #[test]
    fn deletion_and_overwrite_annotations_warn_external_clients() {
        let tools = definitions(false);
        for name in [
            "delete_video",
            "delete_archive",
            "generate_archive_subtitle",
        ] {
            let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
            assert_eq!(tool["annotations"]["readOnlyHint"], false);
            assert_eq!(tool["annotations"]["destructiveHint"], true);
            assert!(!tool["description"]
                .as_str()
                .unwrap()
                .contains("after user confirmation"));
        }
    }

    #[test]
    fn mcp_platform_schemas_cover_supported_platforms_including_nested_clips() {
        let tools = definitions(false);
        let expected = json!(PlatformType::ALL.map(|platform| platform.as_str()));
        let recorder = tools
            .iter()
            .find(|tool| tool["name"] == "add_recorder")
            .unwrap();
        let clip = tools
            .iter()
            .find(|tool| tool["name"] == "clip_range")
            .unwrap();
        assert_eq!(
            recorder["inputSchema"]["properties"]["platform"]["enum"],
            expected
        );
        let removal = tools
            .iter()
            .find(|tool| tool["name"] == "remove_account")
            .unwrap();
        assert_eq!(
            removal["inputSchema"]["properties"]["uid"]["type"],
            "string"
        );
        assert_eq!(
            clip["inputSchema"]["properties"]["clip_range_params"]["properties"]["platform"]
                ["enum"],
            expected
        );
        // Extending the MCP catalog must not widen the existing assistant's
        // tool schemas or alter its confirmation behavior.
        let assistant = agent::bsr_tool_definitions();
        let recorder = assistant
            .iter()
            .find(|schema| schema["properties"]["action"]["const"] == "add_recorder")
            .unwrap();
        assert_eq!(
            recorder["properties"]["args"]["properties"]["platform"]["enum"],
            json!(["bilibili", "douyin", "douyu"])
        );
    }

    #[test]
    fn mcp_clip_schema_omits_unused_reason_and_preserves_assistant_schema() {
        let tools = definitions(false);
        let clip = tools
            .iter()
            .find(|tool| tool["name"] == "clip_range")
            .unwrap();
        assert!(clip["inputSchema"]["properties"].get("reason").is_none());
        assert_eq!(
            clip["inputSchema"]["required"],
            json!(["clip_range_params"])
        );
        let assistant = agent::bsr_tool_definitions();
        let clip = assistant
            .iter()
            .find(|schema| schema["properties"]["action"]["const"] == "clip_range")
            .unwrap();
        assert!(clip["properties"]["args"]["properties"]
            .get("reason")
            .is_some());
        assert_eq!(
            clip["properties"]["args"]["required"],
            json!(["reason", "clip_range_params"])
        );
    }

    #[test]
    fn subtitle_styles_accept_editor_defaults_and_normal_ass_values() {
        for style in [
            "",
            "FontName=Arial,FontSize=18,PrimaryColour=&HFFFFFF,OutlineColour=&H000000,Outline=2,Alignment=2,MarginV=20,MarginL=20,MarginR=20",
            "FontName=Noto Sans CJK SC,FontSize=24.5,Outline=0.5,Alignment=9",
            "FontName=思源黑体 (Regular),PrimaryColour=&H00ffffff&,OutlineColour=&h80000000,Alignment=10,MarginL=0",
            "FontName=Source-Han_Sans.2,Outline=4,Alignment=11",
        ] {
            assert!(validate_subtitle_style(style).is_ok(), "Rejected valid style: {style}");
            assert!(validate_arguments("encode_video_subtitle", &json!({ "id": 1, "srt_style": style })).is_ok());
        }
    }

    #[test]
    fn subtitle_styles_reject_filter_injection_paths_and_unknown_overrides() {
        for style in [
            "FontName=Arial',movie=/etc/passwd[out];[in]overlay[out]",
            "FontName=Arial\\',movie=http://127.0.0.1/private",
            "FontName=Arial\"",
            "FontName=Arial;movie=secret",
            "FontName=Arial[other]",
            "FontName=http://127.0.0.1/private",
            "FontName=file:/etc/passwd",
            "FontName=../secret/font.ttf",
            "FontName=C:\\Windows\\secret.ttf",
            "FontName=Arial\nFontSize=18",
            "FontName=Arial\0",
            "FontName=Arial,filename=/etc/passwd",
            "FontName=Arial,fontsdir=/private/fonts",
            "Bold=1",
            "FontName=Arial,FontName=Other",
            "FontName=Arial,",
            "PrimaryColour=&HFFFFFF:filename=secret",
            "OutlineColour=&HFFFFFF'",
            "FontSize=18:enable=1",
            "MarginV=20,drawtext=text=secret",
        ] {
            assert!(
                validate_subtitle_style(style).is_err(),
                "Accepted unsafe style: {style}"
            );
            assert!(validate_arguments(
                "encode_video_subtitle",
                &json!({ "id": 1, "srt_style": style })
            )
            .is_err());
        }
    }

    #[test]
    fn subtitle_style_values_must_follow_supported_ass_field_grammars() {
        for style in [
            "FontName=",
            "FontSize=0",
            "FontSize=NaN",
            "FontSize=1e6",
            "Outline=-1",
            "Outline=5",
            "Alignment=0",
            "Alignment=12",
            "Alignment=2.5",
            "MarginV=-10",
            "MarginL=20.5",
            "MarginR=2147483648",
            "PrimaryColour=#FFFFFF",
            "PrimaryColour=&H123",
            "PrimaryColour=&HGGGGGG",
            "OutlineColour=&HFFFFFFFFFF",
        ] {
            assert!(
                validate_subtitle_style(style).is_err(),
                "Accepted invalid ASS value: {style}"
            );
        }
        assert!(validate_subtitle_style(&format!("FontName={}", "A".repeat(2048))).is_err());
    }

    #[test]
    fn validates_pagination_and_highlight_inputs_before_loading_data() {
        assert!(validate_arguments("get_archives", &json!({ "offset": 0, "limit": 20 })).is_ok());
        for args in [
            json!({ "offset": -1, "limit": 20 }),
            json!({ "offset": 0, "limit": 0 }),
            json!({ "offset": 0, "limit": 1001 }),
        ] {
            assert!(validate_arguments("get_archives", &args).is_err());
        }
        assert!(validate_arguments(
            "analyze_danmu_highlights",
            &json!({ "time_window": 30, "min_density": 5 })
        )
        .is_ok());
        assert!(validate_arguments(
            "analyze_danmu_highlights",
            &json!({ "time_window": 0.0001, "min_density": 5 })
        )
        .is_err());
        assert!(validate_arguments(
            "search_danmu_keywords",
            &json!({ "keywords": [""], "context_seconds": 0 })
        )
        .is_err());
        assert!(validate_arguments(
            "search_danmu_keywords",
            &json!({ "keywords": ["高光"], "context_seconds": -1 })
        )
        .is_err());
    }

    #[test]
    fn validates_frames_with_defaults_and_limits() {
        assert!(validate_arguments("extract_video_frames", &json!({ "video_id": 1 })).is_ok());
        assert!(validate_arguments("extract_video_frames", &json!({ "max_frames": 0 })).is_err());
        assert!(validate_arguments("extract_video_frames", &json!({ "max_frames": 11 })).is_err());
        assert!(
            validate_arguments("extract_video_frames", &json!({ "timestamps": [-1] })).is_err()
        );
        assert!(validate_arguments(
            "extract_video_frames",
            &json!({ "timestamps": [0, 2.5], "max_frames": 2 })
        )
        .is_ok());
    }

    #[test]
    fn clip_ranges_require_valid_order_and_integral_local_offset() {
        let mut args = json!({ "clip_range_params": { "room_id": "1", "live_id": "2", "platform": "bilibili", "title": "Clip", "note": "", "cover": "", "danmu": false, "fix_encoding": false, "local_offset": 0, "ranges": [{ "start": 10, "end": 20 }] } });
        assert!(validate_arguments("clip_range", &args).is_ok());
        args["clip_range_params"]["ranges"][0]["end"] = json!(9);
        assert!(validate_arguments("clip_range", &args).is_err());
        args["clip_range_params"]["ranges"][0]["end"] = json!(20);
        args["clip_range_params"]["local_offset"] = json!(0.5);
        assert!(validate_arguments("clip_range", &args).is_err());
    }

    #[test]
    fn workflow_tools_cover_projects_and_drafts_but_never_post() {
        let tools = definitions(false);
        let names: HashSet<&str> = tools
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        for name in [
            "get_task",
            "cancel_task",
            "generate_materials",
            "get_projects",
            "get_project",
            "create_project",
            "set_project_clips",
            "update_project_info",
            "update_project_output_settings",
            "delete_project",
            "get_video_typelist",
            "get_uploads",
            "get_upload",
            "create_upload_draft",
            "update_upload_draft",
            "update_upload_danmu",
            "delete_upload",
        ] {
            assert!(names.contains(name), "missing {name}");
        }
        for name in ["post_upload", "upload_procedure", "post_video_to_bilibili"] {
            assert!(!names.contains(name));
        }
        let readonly: HashSet<String> = definitions(true)
            .iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
            .collect();
        for name in [
            "get_task",
            "get_projects",
            "get_project",
            "get_uploads",
            "get_upload",
            "get_video_typelist",
        ] {
            assert!(readonly.contains(name), "read-only mode lacks {name}");
        }
        for name in [
            "generate_materials",
            "create_project",
            "set_project_clips",
            "create_upload_draft",
            "delete_upload",
            "cancel_task",
        ] {
            assert!(!readonly.contains(name), "read-only mode offers {name}");
            assert!(resolve_action(name, true).is_err());
        }
        for name in ["set_project_clips", "delete_project", "delete_upload"] {
            let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
            assert_eq!(tool["annotations"]["destructiveHint"], true);
        }
    }

    #[test]
    fn clip_parameters_with_defaults_may_be_omitted() {
        let tools = definitions(false);
        for name in ["clip_range", "generate_materials"] {
            let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
            let params = &tool["inputSchema"]["properties"]["clip_range_params"];
            assert_eq!(
                params["required"],
                json!(["room_id", "live_id", "ranges", "title", "platform"])
            );
            assert_eq!(params["properties"]["local_offset"]["type"], "integer");
            assert_eq!(params["properties"]["ranges"]["minItems"], 1);
        }
        let args = json!({ "clip_range_params": { "room_id": "1", "live_id": "2", "platform": "bilibili", "title": "Clip", "ranges": [{ "start": 10, "end": 20 }] } });
        assert!(validate_arguments("generate_materials", &args).is_ok());
        let params = clip_range_params(&args).unwrap();
        assert!(!params.danmu && !params.fix_encoding);
        assert_eq!(params.local_offset, 0);
        assert!(params.cover.is_empty() && params.note.is_empty());
        let mut args = args;
        args["clip_range_params"]["ranges"] = json!([]);
        assert!(validate_arguments("generate_materials", &args).is_err());
    }

    #[test]
    fn danmu_windows_and_project_clips_are_validated() {
        assert!(validate_arguments("get_danmu_record", &json!({})).is_ok());
        assert!(
            validate_arguments("get_danmu_record", &json!({ "start": 60, "end": 120 })).is_ok()
        );
        assert!(validate_arguments("get_danmu_record", &json!({ "end": 30 })).is_ok());
        assert!(validate_arguments("get_danmu_record", &json!({ "start": -1 })).is_err());
        assert!(
            validate_arguments("get_danmu_record", &json!({ "start": 60, "end": 60 })).is_err()
        );
        let clips = |clips: Value| json!({ "id": 1, "clips": clips });
        assert!(
            validate_arguments("set_project_clips", &clips(json!([{ "material_id": 3 }]))).is_ok()
        );
        assert!(validate_arguments(
            "set_project_clips",
            &clips(
                json!([{ "clip_id": "a", "material_id": 3, "start": 2, "end": 9.5, "volume": 0.5 }])
            )
        )
        .is_ok());
        assert!(validate_arguments("set_project_clips", &clips(json!([]))).is_err());
        assert!(validate_arguments(
            "set_project_clips",
            &clips(json!([{ "material_id": 3, "start": 5, "end": 5 }]))
        )
        .is_err());
    }

    #[test]
    fn gui_only_tools_are_not_advertised_by_headless_builds() {
        let names = definitions(false)
            .into_iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect::<HashSet<_>>();
        for name in [
            "get_video_metadata",
            "merge_videos",
            "extract_video_audio",
            "get_archive_metadata",
        ] {
            assert_eq!(names.contains(name), cfg!(feature = "gui"));
        }
    }
}
