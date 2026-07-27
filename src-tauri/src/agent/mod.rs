//! Server-side AI assistant powered by [Rig](https://github.com/0xPlaygrounds/rig).
//!
//! The browser supplies only a transient model configuration and conversation
//! transcript.  Model requests and BSR tool calls happen in this process, so
//! browser code never has to implement an agent loop or invoke privileged
//! Tauri commands on the model's behalf.

use std::{
    fmt,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use rig_core::{
    client::{CompletionClient, Nothing},
    completion::{Chat, Message},
    message::{AssistantContent, ToolCall, ToolFunction},
    providers::{ollama, openai},
    tool::Tool,
    OneOrMany,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[cfg(feature = "gui")]
use tauri::State as TauriState;

use crate::{database::Database, recorder_manager::RecorderManager, state::State, state_type};

const PROMPT: &str = r#"
你是 BiliBili ShadowReplay（BSR）的虚拟助手小轴。你喜欢橘子，并适度使用 emoji。
BSR 用 Recorder 表示监控的直播间，Archive 表示缓存的录播，Video/Clip 表示从 Archive 制作的视频。

你可以用 bsr 工具读取和管理 BSR 数据。剪辑高光时，先读取 Archive、字幕和弹幕，再交叉验证字幕、弹幕密度和关键词；不要臆造时间点。所有时长尽量使用中文可读形式，结果优先以 Markdown 表格展示。

只读工具会立即执行。删除、上传、修改配置、生成文件或启动外部操作等工具会返回 confirmation_required=true，此时你必须立即向用户清楚说明待执行的工具及参数，然后停止，不得重复调用该工具、继续调用其他工具或声称操作已完成。前端会让用户手动确认或拒绝；收到对应 tool result 后，再根据实际结果继续回复。每次只申请一个需要确认的操作。
"#;

static TOOL_CALL_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRequest {
    pub provider: String,
    pub endpoint: String,
    pub api_key: Option<String>,
    pub model: String,
    /// Serialized UI messages. Keeping this wire type deliberately simple
    /// avoids coupling the Rust backend to frontend message implementations.
    #[serde(default)]
    pub messages: Vec<AgentMessage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessage {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub tool_calls: Vec<AgentToolCall>,
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AgentToolCall {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentResponse {
    pub content: String,
    #[serde(default)]
    pub tool_calls: Vec<ExecutedToolCall>,
    pub error: Option<String>,
}

/// A tool invocation observed by the server-side Rig agent. Read-only calls are
/// executed here; calls with `executed == false` are requests for explicit UI
/// confirmation and must be executed at most once by the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutedToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
    pub executed: bool,
    pub error: Option<String>,
}

#[derive(Clone)]
struct BsrTool {
    db: Arc<Database>,
    recorder_manager: Arc<RecorderManager>,
    calls: Arc<Mutex<Vec<ExecutedToolCall>>>,
}

#[derive(Debug, Deserialize)]
struct BsrToolArgs {
    action: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug)]
struct BsrToolError(String);

impl fmt::Display for BsrToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BsrToolError {}

impl BsrTool {
    fn tool_call_id() -> String {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sequence = TOOL_CALL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        format!("bsr-{timestamp}-{sequence}")
    }

    fn requires_confirmation(action: &str) -> bool {
        matches!(
            action,
            "remove_account"
                | "add_recorder"
                | "remove_recorder"
                | "delete_archive"
                | "delete_archives"
                | "delete_background_task"
                | "get_video_cover"
                | "delete_video"
                | "get_video_typelist"
                | "get_video_subtitle"
                | "generate_video_subtitle"
                | "encode_video_subtitle"
                | "post_video_to_bilibili"
                | "clip_range"
                | "generic_ffmpeg_command"
                | "open_clip"
                | "list_folder"
                | "generate_archive_subtitle"
                | "extract_video_frames"
                | "get_video_metadata"
                | "merge_videos"
                | "extract_video_audio"
                | "get_archive_metadata"
        )
    }

    fn action_schema(
        action: &str,
        description: &str,
        properties: Value,
        required: &[&str],
    ) -> Value {
        let mut args_schema = json!({
            "type": "object",
            "properties": properties,
            "additionalProperties": false
        });
        if !required.is_empty() {
            args_schema["required"] = json!(required);
        }
        json!({
            "type": "object",
            "description": description,
            "properties": {
                "action": { "const": action },
                "args": args_schema
            },
            "required": ["action", "args"],
            "additionalProperties": false
        })
    }

    fn string_arg(args: &Value, name: &str) -> Result<String, BsrToolError> {
        args.get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| BsrToolError(format!("Missing string argument: {name}")))
    }

    fn i64_arg(args: &Value, name: &str) -> Result<i64, BsrToolError> {
        args.get(name)
            .and_then(Value::as_i64)
            .ok_or_else(|| BsrToolError(format!("Missing integer argument: {name}")))
    }

    fn f64_arg(args: &Value, name: &str) -> Result<f64, BsrToolError> {
        args.get(name)
            .and_then(Value::as_f64)
            .ok_or_else(|| BsrToolError(format!("Missing number argument: {name}")))
    }

    async fn execute(&self, input: BsrToolArgs) -> Result<Value, BsrToolError> {
        let args = &input.args;
        match input.action.as_str() {
            "get_accounts" => {
                let accounts = self
                    .db
                    .get_accounts()
                    .await
                    .map_err(|e| BsrToolError(e.to_string()))?;
                // Cookies must never be exposed to the LLM provider.
                Ok(
                    json!({ "accounts": accounts.into_iter().map(|mut account| { account.cookies = "********".into(); account }).collect::<Vec<_>>() }),
                )
            }
            "get_recorder_list" => Ok(serde_json::to_value(
                self.recorder_manager.get_recorder_list().await,
            )
            .unwrap_or(Value::Null)),
            "get_recorder_info" => {
                use recorder::platforms::PlatformType;
                use std::str::FromStr;
                let platform = PlatformType::from_str(&Self::string_arg(args, "platform")?)
                    .map_err(|e| BsrToolError(e.to_string()))?;
                let room_id = Self::string_arg(args, "room_id")?;
                let info = self
                    .recorder_manager
                    .get_recorder_info(platform, &room_id)
                    .await
                    .ok_or_else(|| BsrToolError("Recorder not found".into()))?;
                Ok(serde_json::to_value(info).unwrap_or(Value::Null))
            }
            "get_archives" => {
                let rows = self
                    .recorder_manager
                    .get_archives(
                        &Self::string_arg(args, "room_id")?,
                        Self::i64_arg(args, "offset")?,
                        Self::i64_arg(args, "limit")?,
                    )
                    .await
                    .map_err(|e| BsrToolError(e.to_string()))?;
                Ok(json!({ "archives": rows }))
            }
            "get_archive" => {
                let row = self
                    .recorder_manager
                    .get_archive(
                        &Self::string_arg(args, "room_id")?,
                        &Self::string_arg(args, "live_id")?,
                    )
                    .await
                    .map_err(|e| BsrToolError(e.to_string()))?;
                Ok(serde_json::to_value(row).unwrap_or(Value::Null))
            }
            "get_recent_record" => {
                let rows = self
                    .db
                    .get_recent_record(
                        &Self::string_arg(args, "room_id")?,
                        Self::i64_arg(args, "offset")?,
                        Self::i64_arg(args, "limit")?,
                    )
                    .await
                    .map_err(|e| BsrToolError(e.to_string()))?;
                Ok(json!({ "records": rows }))
            }
            "get_recent_record_all" => {
                let rows = self
                    .db
                    .get_recent_record(
                        "",
                        Self::i64_arg(args, "offset")?,
                        Self::i64_arg(args, "limit")?,
                    )
                    .await
                    .map_err(|e| BsrToolError(e.to_string()))?;
                Ok(json!({ "records": rows }))
            }
            "get_videos" => Ok(
                json!({ "videos": self.db.get_videos(&Self::string_arg(args, "room_id")?).await.map_err(|e| BsrToolError(e.to_string()))? }),
            ),
            "get_all_videos" => Ok(
                json!({ "videos": self.db.get_all_videos().await.map_err(|e| BsrToolError(e.to_string()))? }),
            ),
            "get_video" => Ok(
                json!({ "video": self.db.get_video(Self::i64_arg(args, "id")?).await.map_err(|e| BsrToolError(e.to_string()))? }),
            ),
            "get_background_tasks" | "get_tasks" => Ok(
                json!({ "tasks": self.db.get_tasks().await.map_err(|e| BsrToolError(e.to_string()))? }),
            ),
            "get_archive_subtitle" => {
                use recorder::platforms::PlatformType;
                use std::str::FromStr;
                let platform = PlatformType::from_str(&Self::string_arg(args, "platform")?)
                    .map_err(|e| BsrToolError(e.to_string()))?;
                let text = self
                    .recorder_manager
                    .get_archive_subtitle(
                        platform,
                        &Self::string_arg(args, "room_id")?,
                        &Self::string_arg(args, "live_id")?,
                    )
                    .await
                    .map_err(|e| BsrToolError(e.to_string()))?;
                Ok(json!({ "subtitle": text }))
            }
            "get_danmu_record" => {
                use recorder::platforms::PlatformType;
                use std::str::FromStr;
                let platform = PlatformType::from_str(&Self::string_arg(args, "platform")?)
                    .map_err(|e| BsrToolError(e.to_string()))?;
                let rows = self
                    .recorder_manager
                    .load_danmus(
                        platform,
                        &Self::string_arg(args, "room_id")?,
                        &Self::string_arg(args, "live_id")?,
                    )
                    .await
                    .map_err(|e| BsrToolError(e.to_string()))?;
                let records = rows
                    .into_iter()
                    .map(|row| {
                        json!({
                            "ts": row.ts as f64 / 1000.0,
                            "content": row.content,
                        })
                    })
                    .collect::<Vec<_>>();
                Ok(json!({ "danmu_record": records }))
            }
            "analyze_danmu_highlights" => {
                use recorder::platforms::PlatformType;
                use std::str::FromStr;
                let platform = PlatformType::from_str(&Self::string_arg(args, "platform")?)
                    .map_err(|e| BsrToolError(e.to_string()))?;
                let window = Self::f64_arg(args, "time_window")?;
                if window <= 0.0 {
                    return Err(BsrToolError("time_window must be positive".into()));
                }
                let minimum = Self::i64_arg(args, "min_density")?;
                let minimum = usize::try_from(minimum).map_err(|_| {
                    BsrToolError("min_density must be a non-negative integer".into())
                })?;
                let rows = self
                    .recorder_manager
                    .load_danmus(
                        platform,
                        &Self::string_arg(args, "room_id")?,
                        &Self::string_arg(args, "live_id")?,
                    )
                    .await
                    .map_err(|e| BsrToolError(e.to_string()))?;
                let max_ts = rows.iter().map(|row| row.ts).max().unwrap_or(0) as f64 / 1000.0;
                let highlights = (0..(max_ts / window).ceil() as usize).filter_map(|index| {
                    let start = index as f64 * window;
                    let end = ((index + 1) as f64 * window).min(max_ts);
                    let comments = rows.iter().filter(|row| (row.ts as f64 / 1000.0) >= start && (row.ts as f64 / 1000.0) < end).collect::<Vec<_>>();
                    (comments.len() >= minimum).then(|| json!({
                        "start_time": start, "end_time": end, "comment_count": comments.len(),
                        "density": comments.len() as f64 / window,
                        "sample_comments": comments.iter().take(5).map(|row| row.content.clone()).collect::<Vec<_>>()
                    }))
                }).collect::<Vec<_>>();
                Ok(json!({ "highlights": highlights }))
            }
            "search_danmu_keywords" => {
                use recorder::platforms::PlatformType;
                use std::str::FromStr;
                let platform = PlatformType::from_str(&Self::string_arg(args, "platform")?)
                    .map_err(|e| BsrToolError(e.to_string()))?;
                let keywords = args
                    .get("keywords")
                    .and_then(Value::as_array)
                    .ok_or_else(|| BsrToolError("Missing keywords".into()))?
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>();
                let context = Self::f64_arg(args, "context_seconds")?;
                let rows = self
                    .recorder_manager
                    .load_danmus(
                        platform,
                        &Self::string_arg(args, "room_id")?,
                        &Self::string_arg(args, "live_id")?,
                    )
                    .await
                    .map_err(|e| BsrToolError(e.to_string()))?;
                let matches = rows.iter().flat_map(|row| keywords.iter().filter(move |keyword| row.content.contains(**keyword)).take(1).map(move |keyword| {
                    let timestamp = row.ts as f64 / 1000.0;
                    json!({ "timestamp": timestamp, "content": row.content, "keyword": keyword, "context_start": (timestamp - context).max(0.0), "context_end": timestamp + context })
                })).collect::<Vec<_>>();
                Ok(json!({ "matches": matches }))
            }
            other => Err(BsrToolError(format!("Unknown BSR action: {other}"))),
        }
    }
}

impl Tool for BsrTool {
    const NAME: &'static str = "bsr";
    type Error = BsrToolError;
    type Args = BsrToolArgs;
    type Output = Value;

    fn description(&self) -> String {
        "Read, analyse and manage BSR. Read-only actions execute immediately. Actions that change state, generate files, access local files through the browser, or start external operations return a confirmation request that the user must approve in the UI.".into()
    }

    fn parameters(&self) -> Value {
        let platform = || json!({ "type": "string", "enum": ["bilibili", "douyin"] });
        let string = || json!({ "type": "string" });
        let integer = || json!({ "type": "integer" });
        let number = || json!({ "type": "number" });
        let boolean = || json!({ "type": "boolean" });
        let transition = || {
            json!({
                "type": "string",
                "enum": ["none", "fade", "dissolve", "wipeleft", "wiperight", "slideup", "slidedown"]
            })
        };
        let actions = vec![
            Self::action_schema(
                "get_accounts",
                "Get configured accounts with cookies redacted.",
                json!({}),
                &[],
            ),
            Self::action_schema(
                "remove_account",
                "Remove an account after user confirmation.",
                json!({ "platform": platform(), "uid": integer() }),
                &["platform", "uid"],
            ),
            Self::action_schema(
                "add_recorder",
                "Add a monitored live room after user confirmation.",
                json!({ "platform": platform(), "room_id": string(), "extra": string() }),
                &["platform", "room_id", "extra"],
            ),
            Self::action_schema(
                "remove_recorder",
                "Stop monitoring a live room after user confirmation.",
                json!({ "platform": platform(), "room_id": string() }),
                &["platform", "room_id"],
            ),
            Self::action_schema(
                "get_recorder_list",
                "List all monitored live rooms.",
                json!({}),
                &[],
            ),
            Self::action_schema(
                "get_recorder_info",
                "Get live-room information.",
                json!({ "platform": platform(), "room_id": string() }),
                &["platform", "room_id"],
            ),
            Self::action_schema(
                "get_archives",
                "List archives for a room.",
                json!({ "room_id": string(), "offset": integer(), "limit": integer() }),
                &["room_id", "offset", "limit"],
            ),
            Self::action_schema(
                "get_archive",
                "Get one archive.",
                json!({ "room_id": string(), "live_id": string() }),
                &["room_id", "live_id"],
            ),
            Self::action_schema(
                "delete_archive",
                "Delete one archive after user confirmation.",
                json!({ "platform": platform(), "room_id": string(), "live_id": string() }),
                &["platform", "room_id", "live_id"],
            ),
            Self::action_schema(
                "delete_archives",
                "Delete multiple archives after user confirmation.",
                json!({ "platform": platform(), "room_id": string(), "live_ids": { "type": "array", "items": string() } }),
                &["platform", "room_id", "live_ids"],
            ),
            Self::action_schema(
                "get_background_tasks",
                "List background tasks.",
                json!({}),
                &[],
            ),
            Self::action_schema(
                "delete_background_task",
                "Delete a background task after user confirmation.",
                json!({ "id": string() }),
                &["id"],
            ),
            Self::action_schema(
                "get_videos",
                "List videos for a room.",
                json!({ "room_id": string() }),
                &["room_id"],
            ),
            Self::action_schema(
                "get_all_videos",
                "List videos from all rooms.",
                json!({}),
                &[],
            ),
            Self::action_schema(
                "get_video",
                "Get one video.",
                json!({ "id": integer() }),
                &["id"],
            ),
            Self::action_schema(
                "get_video_cover",
                "Get a video cover after user confirmation because the result is handled locally.",
                json!({ "id": integer() }),
                &["id"],
            ),
            Self::action_schema(
                "delete_video",
                "Delete a video after user confirmation.",
                json!({ "id": integer() }),
                &["id"],
            ),
            Self::action_schema(
                "get_video_typelist",
                "Get Bilibili video categories after user confirmation.",
                json!({}),
                &[],
            ),
            Self::action_schema(
                "get_video_subtitle",
                "Get a generated video subtitle after user confirmation.",
                json!({ "id": integer() }),
                &["id"],
            ),
            Self::action_schema(
                "generate_video_subtitle",
                "Generate or overwrite a video subtitle after user confirmation.",
                json!({ "id": integer() }),
                &["id"],
            ),
            Self::action_schema(
                "encode_video_subtitle",
                "Burn subtitles into a video after user confirmation.",
                json!({ "id": integer(), "srt_style": string() }),
                &["id", "srt_style"],
            ),
            Self::action_schema(
                "post_video_to_bilibili",
                "Upload a video to Bilibili after user confirmation.",
                json!({ "uid": integer(), "room_id": string(), "video_id": integer(), "title": string(), "desc": string(), "tag": string(), "tid": integer() }),
                &["uid", "room_id", "video_id", "title", "desc", "tag", "tid"],
            ),
            Self::action_schema(
                "get_danmu_record",
                "Load danmu for an archive. Each ts value is seconds from the archive start.",
                json!({ "platform": platform(), "room_id": string(), "live_id": string() }),
                &["platform", "room_id", "live_id"],
            ),
            Self::action_schema(
                "clip_range",
                "Create a video from one or more archive ranges after user confirmation.",
                json!({
                    "reason": string(),
                    "clip_range_params": {
                        "type": "object",
                        "properties": {
                            "room_id": string(), "live_id": string(),
                            "ranges": { "type": "array", "items": { "type": "object", "properties": { "start": number(), "end": number() }, "required": ["start", "end"], "additionalProperties": false } },
                            "danmu": boolean(), "local_offset": number(), "title": string(), "note": string(), "cover": string(), "platform": platform(), "fix_encoding": boolean(), "transition": transition()
                        },
                        "required": ["room_id", "live_id", "ranges", "danmu", "local_offset", "title", "note", "cover", "platform", "fix_encoding"],
                        "additionalProperties": false
                    }
                }),
                &["reason", "clip_range_params"],
            ),
            Self::action_schema(
                "get_recent_record",
                "List recent archives for a room.",
                json!({ "room_id": string(), "offset": integer(), "limit": integer() }),
                &["room_id", "offset", "limit"],
            ),
            Self::action_schema(
                "get_recent_record_all",
                "List recent archives from all rooms.",
                json!({ "offset": integer(), "limit": integer() }),
                &["offset", "limit"],
            ),
            Self::action_schema(
                "generic_ffmpeg_command",
                "Run ffmpeg arguments after user confirmation.",
                json!({ "args": { "type": "array", "items": string() } }),
                &["args"],
            ),
            Self::action_schema(
                "open_clip",
                "Open a local video preview after user confirmation.",
                json!({ "video_id": integer() }),
                &["video_id"],
            ),
            Self::action_schema(
                "list_folder",
                "List a local folder after user confirmation.",
                json!({ "path": string() }),
                &["path"],
            ),
            Self::action_schema(
                "get_archive_subtitle",
                "Get an archive subtitle.",
                json!({ "platform": platform(), "room_id": string(), "live_id": string() }),
                &["platform", "room_id", "live_id"],
            ),
            Self::action_schema(
                "generate_archive_subtitle",
                "Generate or overwrite an archive subtitle after user confirmation.",
                json!({ "platform": platform(), "room_id": string(), "live_id": string() }),
                &["platform", "room_id", "live_id"],
            ),
            Self::action_schema(
                "extract_video_frames",
                "Extract local video frames after user confirmation.",
                json!({ "video_id": integer(), "timestamps": { "type": "array", "items": number() }, "max_frames": integer() }),
                &["video_id"],
            ),
            Self::action_schema(
                "get_video_metadata",
                "Inspect local video metadata after user confirmation.",
                json!({ "video_id": integer() }),
                &["video_id"],
            ),
            Self::action_schema(
                "analyze_danmu_highlights",
                "Find high-engagement time windows from danmu density.",
                json!({ "platform": platform(), "room_id": string(), "live_id": string(), "time_window": number(), "min_density": integer() }),
                &[
                    "platform",
                    "room_id",
                    "live_id",
                    "time_window",
                    "min_density",
                ],
            ),
            Self::action_schema(
                "search_danmu_keywords",
                "Find keyword matches and timestamps in danmu.",
                json!({ "platform": platform(), "room_id": string(), "live_id": string(), "keywords": { "type": "array", "items": string() }, "context_seconds": number() }),
                &[
                    "platform",
                    "room_id",
                    "live_id",
                    "keywords",
                    "context_seconds",
                ],
            ),
            Self::action_schema(
                "merge_videos",
                "Merge videos after user confirmation.",
                json!({ "video_ids": { "type": "array", "items": integer() }, "output_title": string(), "output_note": string(), "transition": transition() }),
                &["video_ids", "output_title", "output_note"],
            ),
            Self::action_schema(
                "extract_video_audio",
                "Extract audio from a video after user confirmation.",
                json!({ "video_id": integer() }),
                &["video_id"],
            ),
            Self::action_schema(
                "get_archive_metadata",
                "Inspect local archive metadata after user confirmation.",
                json!({ "platform": platform(), "room_id": string(), "live_id": string() }),
                &["platform", "room_id", "live_id"],
            ),
        ];
        json!({
            "type": "object",
            "oneOf": actions
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // Rig owns the agent loop, so this is the one reliable boundary where
        // every provider-triggered tool call can be captured for the UI.
        let id = Self::tool_call_id();
        let record_index = {
            let mut calls = self.calls.lock().expect("tool call log mutex poisoned");
            calls.push(ExecutedToolCall {
                id: id.clone(),
                name: args.action.clone(),
                args: args.args.clone(),
                executed: false,
                error: None,
            });
            calls.len() - 1
        };

        if Self::requires_confirmation(&args.action) {
            return Ok(json!({
                "confirmation_required": true,
                "tool_call_id": id,
                "action": args.action,
                "args": args.args,
                "message": "Wait for the user to confirm or reject this operation in the BSR UI."
            }));
        }

        let result = self.execute(args).await;
        let mut calls = self.calls.lock().expect("tool call log mutex poisoned");
        let record = &mut calls[record_index];
        record.executed = true;
        if let Err(error) = &result {
            record.error = Some(error.to_string());
        }
        result
    }
}

fn rig_message(message: &AgentMessage) -> Result<Message, String> {
    match message.role.as_str() {
        "user" => Ok(Message::user(message.content.clone())),
        "assistant" => {
            let mut content = Vec::new();
            if !message.content.is_empty() {
                content.push(AssistantContent::text(message.content.clone()));
            }
            content.extend(message.tool_calls.iter().map(|call| {
                AssistantContent::ToolCall(ToolCall::new(
                    call.id.clone(),
                    ToolFunction::new(
                        BsrTool::NAME.to_owned(),
                        json!({ "action": call.name, "args": call.args }),
                    ),
                ))
            }));
            if content.is_empty() {
                content.push(AssistantContent::text(""));
            }
            Ok(Message::Assistant {
                id: None,
                content: OneOrMany::many(content).expect("assistant content is not empty"),
            })
        }
        "tool" => message
            .tool_call_id
            .as_ref()
            .map(|id| Message::tool_result(id.clone(), message.content.clone()))
            .ok_or_else(|| "Tool message is missing toolCallId".to_owned()),
        "system" => Ok(Message::system(message.content.clone())),
        role => Err(format!("Unsupported message role: {role}")),
    }
}

fn prepare_chat(messages: &[AgentMessage]) -> Result<(Message, Vec<Message>), String> {
    let mut history = messages
        .iter()
        .map(rig_message)
        .collect::<Result<Vec<_>, _>>()?;
    let prompt = history
        .pop()
        .ok_or_else(|| "Conversation must contain at least one message".to_owned())?;
    Ok((prompt, history))
}

async fn openai_chat(
    request: &AgentRequest,
    tool: BsrTool,
    prompt: Message,
    mut history: Vec<Message>,
) -> Result<String, String> {
    let client = openai::Client::builder()
        .api_key(request.api_key.as_deref().unwrap_or_default())
        .base_url(&request.endpoint)
        .build()
        .map_err(|e| e.to_string())?
        // OpenAI-compatible gateways commonly implement Chat Completions, not Responses.
        .completions_api();
    client
        .agent(&request.model)
        .preamble(PROMPT)
        .tool(tool)
        .default_max_turns(8)
        .build()
        .chat(prompt, &mut history)
        .await
        .map_err(|e| e.to_string())
}

async fn ollama_chat(
    request: &AgentRequest,
    tool: BsrTool,
    prompt: Message,
    mut history: Vec<Message>,
) -> Result<String, String> {
    let endpoint = if request.endpoint.trim().is_empty() {
        "http://localhost:11434"
    } else {
        &request.endpoint
    };
    let client = ollama::Client::builder()
        .api_key(Nothing)
        .base_url(endpoint)
        .build()
        .map_err(|e| e.to_string())?;
    client
        .agent(&request.model)
        .preamble(PROMPT)
        .tool(tool)
        .default_max_turns(8)
        .build()
        .chat(prompt, &mut history)
        .await
        .map_err(|e| e.to_string())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn agent_chat(
    state: state_type!(),
    request: AgentRequest,
) -> Result<AgentResponse, String> {
    if request.model.trim().is_empty() {
        return Err("请选择模型".into());
    }
    let tool = BsrTool {
        db: state.db.clone(),
        recorder_manager: state.recorder_manager.clone(),
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let calls = tool.calls.clone();
    let (prompt, history) = prepare_chat(&request.messages)?;
    let result = match request.provider.as_str() {
        "ollama" => ollama_chat(&request, tool, prompt, history).await,
        "openai" => openai_chat(&request, tool, prompt, history).await,
        _ => return Err("Unsupported AI provider".into()),
    };
    let tool_calls = calls.lock().expect("tool call log mutex poisoned").clone();
    match result {
        Ok(content) => Ok(AgentResponse {
            content,
            tool_calls,
            error: None,
        }),
        Err(error) => Ok(AgentResponse {
            content: String::new(),
            tool_calls,
            error: Some(error),
        }),
    }
}
