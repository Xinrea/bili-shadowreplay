//! MCP tools for the steps after cutting materials: projects (切片) and
//! upload drafts (投稿草稿). External clients prepare drafts only; a person
//! reviews and posts them from the uploads page.

use serde_json::{json, Value};

use super::tools::argument;
use crate::{
    handlers::{self, project::ClipRequest},
    project::RenderSettings,
    state::State,
};

fn integer() -> Value {
    json!({ "type": "integer" })
}

fn text(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn id_schema(description: &str) -> Value {
    schema(
        json!({ "id": { "type": "integer", "description": description } }),
        &["id"],
    )
}

/// The upload form fields clients may fill.
fn profile_properties() -> serde_json::Map<String, Value> {
    let Value::Object(properties) = json!({
        "uid": text("Bilibili account that will post it, from get_accounts (platform bilibili)."),
        "title": { "type": "string", "minLength": 1, "description": "Title of the submission." },
        "desc": text("Description."),
        "tag": text("Comma-separated tags."),
        "tid": { "type": "integer", "minimum": 1, "description": "Category (分区) id from get_video_typelist." },
        "dynamic": text("Text of the dynamic (动态) published with it.")
    }) else {
        unreachable!("a JSON object literal")
    };
    properties
}

/// Tool name, description and input schema of each tool defined here.
pub(super) fn definitions(clip_range_params: Value) -> Vec<(&'static str, String, Value)> {
    let mut create_upload = profile_properties();
    create_upload.insert(
        "source".into(),
        json!({ "type": "string", "enum": ["material", "project"], "description": "Post a material (video) as is, or a project rendered from its timeline." }),
    );
    create_upload.insert(
        "source_id".into(),
        json!({ "type": "integer", "description": "Id of the material or project." }),
    );
    let mut update_upload = profile_properties();
    update_upload.insert(
        "id".into(),
        json!({ "type": "integer", "description": "Upload id." }),
    );
    vec![
        (
            "get_task",
            "Get a background task: status is pending, processing, success, failed or cancelled. Long tools return a task_id; once it succeeds, metadata.result holds what it produced.".into(),
            schema(json!({ "id": text("Task id.") }), &["id"]),
        ),
        (
            "cancel_task",
            "Cancel a pending or running background task. Tasks that do not run on the task queue, such as subtitle generation, cannot be cancelled.".into(),
            schema(json!({ "id": text("Task id.") }), &["id"]),
        ),
        (
            "generate_materials",
            "Cut materials from an archive in the background: one material per range, or with merge one material joining them in order. With create_project, also create a project playing the materials in order. Returns a task_id; metadata.result has material_ids and project_id.".into(),
            schema(
                json!({
                    "clip_range_params": clip_range_params,
                    "merge": { "type": "boolean", "description": "Join all ranges into one material (re-encodes). Default false." },
                    "create_project": { "type": "boolean", "description": "Also create a project of the new materials. Default false." }
                }),
                &["clip_range_params"],
            ),
        ),
        (
            "get_projects",
            "List projects (切片), the edits that combine materials, without their timelines.".into(),
            schema(json!({ "room_id": text("Only projects of this room.") }), &[]),
        ),
        (
            "get_project",
            "Get a project with its timeline: clips play back to back in order; start and end are in and out points within each clip's material, in seconds. Subtitles are anchored to clips in material time.".into(),
            id_schema("Project id."),
        ),
        (
            "create_project",
            "Create a project with one full-length clip per material, in order. Clips get their materials' recognized subtitles.".into(),
            schema(
                json!({
                    "material_ids": { "type": "array", "items": integer(), "minItems": 1 },
                    "title": text("Defaults to the first material's title."),
                    "room_id": text("Defaults to the first material's room."),
                    "platform": text("Defaults to the first material's platform.")
                }),
                &["material_ids"],
            ),
        ),
        (
            "set_project_clips",
            "Replace a project's main track with clips, in play order. Give the clip_id of an existing clip of the same material to keep it with its subtitles, texts, stickers, picture-in-picture and sounds; content anchored to clips left out is removed. Clips without subtitles get their material's recognized subtitles. Do not use while the project is open in the editor, which would overwrite the change.".into(),
            schema(
                json!({
                    "id": { "type": "integer", "description": "Project id." },
                    "clips": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "clip_id": text("Existing clip to keep."),
                                "material_id": integer(),
                                "start": { "type": "number", "minimum": 0, "description": "In point within the material, in seconds. Default 0." },
                                "end": { "type": "number", "exclusiveMinimum": 0, "description": "Out point within the material, in seconds. Default the material's end." },
                                "volume": { "type": "number", "minimum": 0, "maximum": crate::project::MAX_VOLUME, "description": "Of the material's sound. Default 1." }
                            },
                            "required": ["material_id"],
                            "additionalProperties": false
                        }
                    }
                }),
                &["id", "clips"],
            ),
        ),
        (
            "update_project_info",
            "Change a project's title, note or both.".into(),
            schema(
                json!({
                    "id": { "type": "integer", "description": "Project id." },
                    "title": { "type": "string", "minLength": 1 },
                    "note": { "type": "string" }
                }),
                &["id"],
            ),
        ),
        (
            "update_project_output_settings",
            "Change how a project is rendered for exports and uploads: burning the materials' danmaku, and the video codec, encoding priority and bitrate.".into(),
            schema(
                json!({
                    "id": { "type": "integer", "description": "Project id." },
                    "danmu": { "type": "boolean", "description": "Burn the materials' danmaku." },
                    "render_settings": {
                        "type": "object",
                        "properties": {
                            "codec": { "type": "string", "enum": ["hevc", "h264"], "default": "hevc" },
                            "priority": { "type": "string", "enum": ["speed", "quality"], "default": "speed" },
                            "bitrate_kbps": { "type": "integer", "minimum": 500, "maximum": 100000, "default": 10000, "description": "Target video bitrate in Kbps." }
                        },
                        "required": ["codec", "priority", "bitrate_kbps"],
                        "additionalProperties": false
                    }
                }),
                &["id"],
            ),
        ),
        (
            "delete_project",
            "Delete a project and its files. Its materials are kept.".into(),
            id_schema("Project id."),
        ),
        (
            "get_video_typelist",
            "Get Bilibili's submission categories (分区) and their ids, using a configured Bilibili account.".into(),
            schema(json!({}), &[]),
        ),
        (
            "get_uploads",
            "List uploads (投稿): drafts and earlier submissions, with their status (draft, exporting, posting, posted, failed) and upload form.".into(),
            schema(json!({}), &[]),
        ),
        (
            "get_upload",
            "Get an upload with its upload form.".into(),
            id_schema("Upload id."),
        ),
        (
            "create_upload_draft",
            "Create an upload draft of a material or project. It is not posted: a person reviews and posts it from BSR's uploads page. Omitted fields use the upload form's defaults; the cover defaults to the source's.".into(),
            schema(Value::Object(create_upload), &["source", "source_id", "title"]),
        ),
        (
            "update_upload_draft",
            "Change fields of an upload draft. Fields left out keep their stored values. Drafts being posted or already posted cannot change.".into(),
            schema(Value::Object(update_upload), &["id"]),
        ),
        (
            "update_upload_danmu",
            "Burn the danmaku into the posted video or not. For a project this changes the project's own setting.".into(),
            schema(
                json!({
                    "id": { "type": "integer", "description": "Upload id." },
                    "danmu": { "type": "boolean" }
                }),
                &["id", "danmu"],
            ),
        ),
        (
            "delete_upload",
            "Delete an upload record and its files. A posted video stays on Bilibili.".into(),
            id_schema("Upload id."),
        ),
    ]
}

/// `value` with the JSON text in `key` parsed, as clients read it.
fn parse_field(mut value: Value, key: &str) -> Value {
    if let Some(parsed) = value[key]
        .as_str()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
    {
        value[key] = parsed;
    }
    value
}

fn to_value(value: impl serde::Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| error.to_string())
}

fn optional<T: serde::de::DeserializeOwned>(args: &Value, name: &str) -> Result<Option<T>, String> {
    args.get(name).map(|_| argument(args, name)).transpose()
}

pub(super) async fn get_projects(state: &State, args: &Value) -> Result<Value, String> {
    let room_id: Option<String> = optional(args, "room_id")?;
    let projects = handlers::project::get_projects(command_state!(state)).await?;
    let projects = projects
        .into_iter()
        .filter(|view| {
            room_id
                .as_ref()
                .is_none_or(|room| *room == view.project.room_id)
        })
        .map(|view| {
            let mut value = to_value(view)?;
            if let Some(fields) = value.as_object_mut() {
                fields.remove("timeline");
                fields.remove("profile");
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(json!({ "projects": projects }))
}

fn project_result(view: handlers::project::ProjectView) -> Result<Value, String> {
    let mut project = parse_field(to_value(view)?, "timeline");
    if let Some(fields) = project.as_object_mut() {
        // Upload details of projects migrated from before uploads had records.
        fields.remove("profile");
    }
    Ok(json!({ "project": project }))
}

pub(super) async fn get_project(state: &State, args: &Value) -> Result<Value, String> {
    let view = handlers::project::get_project(command_state!(state), argument(args, "id")?).await?;
    project_result(view)
}

pub(super) async fn create_project(state: &State, args: &Value) -> Result<Value, String> {
    let material_ids: Vec<i64> = argument(args, "material_ids")?;
    for id in &material_ids {
        state
            .db
            .get_video(*id)
            .await
            .map_err(|_| format!("Material {id} not found"))?;
    }
    let view = handlers::project::create_project(
        command_state!(state),
        optional(args, "title")?.unwrap_or_default(),
        optional(args, "room_id")?.unwrap_or_default(),
        optional(args, "platform")?.unwrap_or_default(),
        material_ids,
    )
    .await?;
    project_result(view)
}

pub(super) async fn set_project_clips(state: &State, args: &Value) -> Result<Value, String> {
    let clips: Vec<ClipRequest> = argument(args, "clips")?;
    let view = handlers::project::set_project_clips(state, argument(args, "id")?, clips).await?;
    project_result(view)
}

pub(super) async fn update_project_info(state: &State, args: &Value) -> Result<Value, String> {
    let title: Option<String> = optional(args, "title")?;
    let note: Option<String> = optional(args, "note")?;
    if title.is_none() && note.is_none() {
        return Err("Provide title, note or both".into());
    }
    let id = argument(args, "id")?;
    state
        .db
        .get_project(id)
        .await
        .map_err(|_| format!("Project {id} not found"))?;
    handlers::project::update_project_info(command_state!(state), id, title, note).await?;
    get_project(state, args).await
}

pub(super) async fn update_project_output_settings(
    state: &State,
    args: &Value,
) -> Result<Value, String> {
    let danmu: Option<bool> = optional(args, "danmu")?;
    let render_settings: Option<RenderSettings> = optional(args, "render_settings")?;
    if danmu.is_none() && render_settings.is_none() {
        return Err("Provide danmu, render_settings or both".into());
    }
    handlers::project::update_project_output_settings(
        command_state!(state),
        argument(args, "id")?,
        danmu,
        render_settings,
    )
    .await?;
    get_project(state, args).await
}

fn upload_result(view: handlers::upload::UploadView) -> Result<Value, String> {
    Ok(json!({ "upload": parse_field(to_value(view)?, "profile") }))
}

pub(super) async fn get_uploads(state: &State) -> Result<Value, String> {
    let uploads = handlers::upload::get_uploads(command_state!(state))
        .await?
        .into_iter()
        .map(|view| Ok(parse_field(to_value(view)?, "profile")))
        .collect::<Result<Vec<_>, String>>()?;
    Ok(json!({ "uploads": uploads }))
}

pub(super) async fn get_upload(state: &State, args: &Value) -> Result<Value, String> {
    upload_result(handlers::upload::get_upload(command_state!(state), argument(args, "id")?).await?)
}

/// Copy the upload form fields given in `args` into `profile`.
fn apply_profile_fields(args: &Value, profile: &mut Value) {
    for key in ["title", "desc", "tag", "tid", "dynamic"] {
        if let Some(value) = args.get(key) {
            profile[key] = value.clone();
        }
    }
}

pub(super) async fn create_upload_draft(state: &State, args: &Value) -> Result<Value, String> {
    let uid: Option<String> = optional(args, "uid")?;
    if let Some(uid) = &uid {
        state
            .db
            .get_account("bilibili", uid)
            .await
            .map_err(|_| format!("No Bilibili account with UID {uid}"))?;
    }
    let mut profile = to_value(recorder::platforms::bilibili::profile::Profile::default())?;
    apply_profile_fields(args, &mut profile);
    let view = handlers::upload::create_upload(
        command_state!(state),
        argument(args, "source")?,
        argument(args, "source_id")?,
        String::new(),
        profile,
    )
    .await?;
    let view = match uid {
        Some(uid) => {
            handlers::upload::patch_upload(state, view.upload.id, Some(&uid), |_| {}).await?
        }
        None => view,
    };
    upload_result(view)
}

pub(super) async fn update_upload_draft(state: &State, args: &Value) -> Result<Value, String> {
    let uid: Option<String> = optional(args, "uid")?;
    let view =
        handlers::upload::patch_upload(state, argument(args, "id")?, uid.as_deref(), |profile| {
            apply_profile_fields(args, profile)
        })
        .await?;
    upload_result(view)
}

pub(super) async fn update_upload_danmu(state: &State, args: &Value) -> Result<Value, String> {
    upload_result(
        handlers::upload::update_upload_danmu(
            command_state!(state),
            argument(args, "id")?,
            argument(args, "danmu")?,
        )
        .await?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_settings_schema_matches_current_render_settings() {
        let (_, _, schema) = definitions(json!({}))
            .into_iter()
            .find(|(name, _, _)| *name == "update_project_output_settings")
            .unwrap();
        let settings = &schema["properties"]["render_settings"];
        assert_eq!(
            settings["required"],
            json!(["codec", "priority", "bitrate_kbps"])
        );
        let properties = &settings["properties"];
        assert_eq!(properties["codec"]["enum"], json!(["hevc", "h264"]));
        assert_eq!(properties["priority"]["enum"], json!(["speed", "quality"]));
        assert_eq!(properties["bitrate_kbps"]["minimum"], 500);
        assert_eq!(properties["bitrate_kbps"]["maximum"], 100000);
        assert!(properties.get("quality").is_none());
        let defaults = serde_json::to_value(RenderSettings::default()).unwrap();
        for field in ["codec", "priority", "bitrate_kbps"] {
            assert_eq!(properties[field]["default"], defaults[field]);
        }
    }

    #[test]
    fn profile_fields_patch_only_what_is_given() {
        let mut profile =
            to_value(recorder::platforms::bilibili::profile::Profile::default()).unwrap();
        apply_profile_fields(&json!({ "title": "高光", "tid": 171 }), &mut profile);
        assert_eq!(profile["title"], "高光");
        assert_eq!(profile["tid"], 171);
        assert_eq!(profile["copyright"], 1);
        assert_eq!(profile["desc"], "");
        assert!(
            serde_json::from_value::<recorder::platforms::bilibili::profile::Profile>(profile)
                .is_ok()
        );
    }

    #[test]
    fn results_show_stored_json_as_objects() {
        let value = parse_field(json!({ "profile": "{\"title\":\"a\"}" }), "profile");
        assert_eq!(value["profile"]["title"], "a");
        // Text that is not JSON stays as it is.
        let value = parse_field(json!({ "profile": "" }), "profile");
        assert_eq!(value["profile"], "");
    }
}
