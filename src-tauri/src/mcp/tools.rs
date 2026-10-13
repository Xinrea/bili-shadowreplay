//! BSR operations exposed to external MCP clients. Shared schemas and data
//! readers keep this catalog aligned with the in-app assistant; writes use the
//! same handlers as the application UI after the user enables the MCP server.

use recorder::platforms::PlatformType;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

#[cfg(feature = "gui")]
use tauri::Manager;

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
    #[cfg(feature = "gui")]
    ExtractVideoFrames,
    #[cfg(feature = "gui")]
    GetVideoMetadata,
    #[cfg(feature = "gui")]
    GetArchiveMetadata,
    #[cfg(feature = "gui")]
    MergeVideos,
    #[cfg(feature = "gui")]
    ExtractVideoAudio,
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
            #[cfg(feature = "gui")]
            "extract_video_frames" => Self::ExtractVideoFrames,
            #[cfg(feature = "gui")]
            "get_video_metadata" => Self::GetVideoMetadata,
            #[cfg(feature = "gui")]
            "get_archive_metadata" => Self::GetArchiveMetadata,
            #[cfg(feature = "gui")]
            "merge_videos" => Self::MergeVideos,
            #[cfg(feature = "gui")]
            "extract_video_audio" => Self::ExtractVideoAudio,
            _ => return None,
        })
    }

    #[cfg(feature = "gui")]
    fn readonly(self) -> bool {
        matches!(
            self,
            Self::Read
                | Self::GetVideoCover
                | Self::GetVideoSubtitle
                | Self::ExtractVideoFrames
                | Self::GetVideoMetadata
                | Self::GetArchiveMetadata
        )
    }

    #[cfg(not(feature = "gui"))]
    fn readonly(self) -> bool {
        matches!(
            self,
            Self::Read | Self::GetVideoCover | Self::GetVideoSubtitle
        )
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
    agent::bsr_tool_definitions()
        .into_iter()
        .filter_map(|schema| {
            let name = schema["properties"]["action"]["const"].as_str()?;
            let action = resolve_action(name, readonly).ok()?;
            let mut input_schema = schema["properties"]["args"].clone();
            include_supported_platforms(&mut input_schema);
            constrain_schema(name, &mut input_schema);
            // Confirmation is provided by external clients. The in-app
            // assistant retains its own, separate confirmation behavior.
            let description = schema["description"]
                .as_str()
                .unwrap_or_default()
                .replace(" after user confirmation because the result is handled locally", "")
                .replace(" after user confirmation", "");
            Some(json!({
                "name": name,
                "description": description,
                "inputSchema": input_schema,
                "annotations": {
                    "readOnlyHint": action.readonly(),
                    "destructiveHint": action.destructive(),
                    "idempotentHint": action.readonly(),
                    "openWorldHint": matches!(action, Action::AddRecorder | Action::RemoveAccount | Action::GenerateArchiveSubtitle | Action::GenerateVideoSubtitle)
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
        "clip_range" => {
            let clip = &mut properties["clip_range_params"]["properties"];
            // The backend stores this offset as an i64, although the older
            // assistant schema allowed arbitrary JSON numbers.
            clip["local_offset"]["type"] = json!("integer");
            clip["ranges"]["minItems"] = json!(1);
            clip["ranges"]["items"]["properties"]["start"]["minimum"] = json!(0);
            clip["ranges"]["items"]["properties"]["end"]["exclusiveMinimum"] = json!(0);
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

fn argument<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T, String> {
    let value = args
        .get(name)
        .ok_or_else(|| format!("Missing argument: {name}"))?;
    serde_json::from_value(value.clone())
        .map_err(|error| format!("Invalid argument {name}: {error}"))
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
        "clip_range" => {
            let params: crate::recorder_manager::ClipRangeParams =
                argument(args, "clip_range_params")?;
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
            let cover =
                handlers::video::get_video_cover(command_state, argument(&args, "id")?).await?;
            Ok(json!({ "cover": cover }))
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
            let event_id = format!("mcp-{}", uuid::Uuid::new_v4());
            let subtitle = handlers::video::generate_video_subtitle(
                command_state,
                event_id.clone(),
                argument(&args, "id")?,
            )
            .await?;
            Ok(json!({ "subtitle": subtitle, "event_id": event_id }))
        }
        Action::EncodeVideoSubtitle => {
            let event_id = format!("mcp-{}", uuid::Uuid::new_v4());
            let video = handlers::video::encode_video_subtitle(
                command_state,
                event_id.clone(),
                argument(&args, "id")?,
                argument(&args, "srt_style")?,
            )
            .await?;
            Ok(json!({ "video": video, "event_id": event_id }))
        }
        Action::ClipRange => {
            let event_id = format!("mcp-{}", uuid::Uuid::new_v4());
            let video = handlers::video::clip_range(
                command_state,
                event_id.clone(),
                argument(&args, "clip_range_params")?,
            )
            .await?;
            Ok(json!({ "video": video, "event_id": event_id }))
        }
        Action::GenerateArchiveSubtitle => {
            let subtitle = handlers::recorder::generate_archive_subtitle(
                command_state,
                argument(&args, "platform")?,
                argument(&args, "room_id")?,
                argument(&args, "live_id")?,
            )
            .await?;
            Ok(json!({ "subtitle": subtitle }))
        }
        #[cfg(feature = "gui")]
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
    fn gui_only_tools_are_not_advertised_by_headless_builds() {
        let names = definitions(false)
            .into_iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect::<HashSet<_>>();
        for name in [
            "extract_video_frames",
            "get_video_metadata",
            "merge_videos",
            "extract_video_audio",
            "get_archive_metadata",
        ] {
            assert_eq!(names.contains(name), cfg!(feature = "gui"));
        }
    }
}
