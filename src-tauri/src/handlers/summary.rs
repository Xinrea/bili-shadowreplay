use std::path::Path;
use std::str::FromStr;

use m3u8_rs::MediaPlaylistType;
use serde::{Deserialize, Serialize};

use crate::database::summary::{RecordSummaryRow, RecordSummaryStatusRow};
use crate::database::task::TaskRow;
use crate::progress::progress_reporter::{EventEmitter, ProgressReporter, ProgressReporterTrait};
use crate::state::State;
use crate::state_type;
use crate::subtitle_generator::item_to_srt;
use crate::task::{Task, TaskPriority};

#[cfg(feature = "gui")]
use tauri::State as TauriState;

const SUMMARY_PREAMBLE: &str = r#"
你是一名严谨的中文直播内容编辑。你只能根据带时间戳的字幕总结内容，不得虚构字幕中不存在的事件或时间点。
输出必须是合法 JSON，不要使用 Markdown 代码块，不要输出 JSON 之外的文字。
精彩片段必须有明确内容价值，时间必须来自字幕，并尽量覆盖一个完整话题。
overview 和 main_topics 是直播内容总结正文，禁止在其中提及时间、时间戳、时间段或精彩片段位置。
所有时间信息只能出现在 highlights 的 start_seconds 和 end_seconds 字段中。
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryHighlight {
    pub title: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub reason: String,
    #[serde(default)]
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SummaryOutput {
    pub overview: String,
    #[serde(default)]
    pub main_topics: Vec<String>,
    #[serde(default)]
    pub highlights: Vec<SummaryHighlight>,
}

fn time_to_seconds(time: &srtparse::Time) -> f64 {
    ((time.hours * 3600 + time.minutes * 60 + time.seconds) as f64)
        + time.milliseconds as f64 / 1000.0
}

fn format_timestamp(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}",
        total / 3600,
        (total % 3600) / 60,
        total % 60
    )
}

fn subtitle_for_llm(items: &[srtparse::Item]) -> String {
    items
        .iter()
        .map(|item| {
            format!(
                "[{} - {}] {}",
                format_timestamp(time_to_seconds(&item.start_time)),
                format_timestamp(time_to_seconds(&item.end_time)),
                item.text.replace('\n', " ").trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_json_response<T: for<'de> Deserialize<'de>>(raw: &str) -> Result<T, String> {
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```");
    let trimmed = trimmed.trim_end_matches("```").trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Ok(value);
    }
    let start = trimmed
        .find('{')
        .ok_or_else(|| "LLM response does not contain JSON".to_string())?;
    let end = trimmed
        .rfind('}')
        .ok_or_else(|| "LLM response does not contain complete JSON".to_string())?;
    serde_json::from_str(&trimmed[start..=end]).map_err(|e| e.to_string())
}

async fn request_summary_json(
    config: &crate::config::LlmConfig,
    prompt: &str,
) -> Result<SummaryOutput, String> {
    let first = crate::agent::llm_prompt(config, SUMMARY_PREAMBLE, prompt).await?;
    match parse_json_response(&first) {
        Ok(output) => Ok(output),
        Err(first_error) => {
            let repair_prompt = format!(
                "请把下面的响应修复成指定结构的合法 JSON。不得增加原响应中没有的事实。\n\n错误：{first_error}\n\n响应：\n{first}\n\n结构：{{\"overview\":\"\",\"main_topics\":[\"\"],\"highlights\":[{{\"title\":\"\",\"start_seconds\":0,\"end_seconds\":0,\"reason\":\"\",\"excerpt\":\"\"}}]}}"
            );
            let repaired =
                crate::agent::llm_prompt(config, SUMMARY_PREAMBLE, &repair_prompt).await?;
            parse_json_response(&repaired)
        }
    }
}

async fn summarize_transcript(
    config: &crate::config::LlmConfig,
    transcript: &str,
    duration: f64,
) -> Result<SummaryOutput, String> {
    if transcript.trim().is_empty() {
        return Err("字幕内容为空".to_string());
    }
    let prompt = format!(
        r#"请完整阅读下面整场直播的字幕，并生成内容充分、具体的直播总结。

要求：
1. overview 应详细概括整场直播，不要只写一两句话；根据实际内容组织为多个自然段，覆盖直播的背景、讨论过程、主要观点、结论和有价值的细节。
2. main_topics 应列出主要话题，每一项都要包含具体内容和观点，而不仅是一个短标题。
3. overview 和 main_topics 中禁止出现任何时间、时间戳、时间范围，也不要提及“精彩片段位于何时”。
4. 精彩内容的时间范围只允许写入 highlights；时间必须严格来自字幕，不能推测。
5. 没有值得推荐的精彩内容时 highlights 返回空数组。
6. overview 和 main_topics 可以使用 Markdown 文本，但整个响应必须保持为合法 JSON。

返回结构：
{{"overview":"详细的直播内容总结","main_topics":["话题及其详细内容"],"highlights":[{{"title":"精彩片段标题","start_seconds":0,"end_seconds":0,"reason":"推荐理由","excerpt":"字幕依据"}}]}}

完整字幕：
{transcript}"#
    );
    let mut output = request_summary_json(config, &prompt).await?;

    output.highlights.retain(|highlight| {
        highlight.start_seconds >= 0.0
            && highlight.end_seconds > highlight.start_seconds
            && highlight.end_seconds <= duration + 1.0
    });
    output.highlights.sort_by(|a, b| {
        a.start_seconds
            .partial_cmp(&b.start_seconds)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(output)
}

fn summary_markdown(output: &SummaryOutput) -> String {
    let mut markdown = format!("# 直播内容总结\n\n{}\n", output.overview.trim());
    if !output.main_topics.is_empty() {
        markdown.push_str("\n## 主要内容\n\n");
        for topic in &output.main_topics {
            markdown.push_str(&format!("- {}\n", topic.trim()));
        }
    }
    markdown
}

async fn create_vod_playlist(playlist: &Path) -> Result<std::path::PathBuf, String> {
    let bytes = tokio::fs::read(playlist)
        .await
        .map_err(|e| format!("读取录播播放列表失败: {e}"))?;
    let (_, mut media_playlist) =
        m3u8_rs::parse_media_playlist(&bytes).map_err(|_| "无法解析录播播放列表".to_string())?;
    if media_playlist.segments.is_empty() {
        return Err("录播播放列表中没有可用分片".to_string());
    }
    media_playlist.end_list = true;
    media_playlist.playlist_type = Some(MediaPlaylistType::Vod);

    let output_path = playlist.with_file_name(format!("summary-{}.m3u8", uuid::Uuid::new_v4()));
    let mut output = Vec::new();
    media_playlist
        .write_to(&mut output)
        .map_err(|e| format!("生成临时 VOD 播放列表失败: {e}"))?;
    tokio::fs::write(&output_path, output)
        .await
        .map_err(|e| format!("写入临时 VOD 播放列表失败: {e}"))?;
    Ok(output_path)
}

async fn run_summary(
    state: State,
    reporter: ProgressReporter,
    platform: String,
    room_id: String,
    live_id: String,
    force: bool,
) -> Result<(), String> {
    let existing = state
        .db
        .get_record_summary(&platform, &room_id, &live_id)
        .await
        .map_err(String::from)?;
    let cached_subtitle = if force {
        None
    } else {
        existing.and_then(|summary| {
            summary
                .subtitle_text
                .zip(summary.subtitle_srt)
                .map(|(text, srt)| (text, srt, summary.source_duration.unwrap_or_default()))
        })
    };

    let (subtitle_text, _subtitle_srt, duration) = if let Some(cached) = cached_subtitle {
        reporter.update("复用已生成字幕").await;
        cached
    } else {
        state
            .db
            .update_record_summary_stage(&platform, &room_id, &live_id, "extracting_audio")
            .await
            .map_err(String::from)?;
        reporter.update("提取完整音频中").await;

        let config = state.config.read().await.clone();
        let playlist = Path::new(&config.cache)
            .join(&platform)
            .join(&room_id)
            .join(&live_id)
            .join("playlist.m3u8");
        if !playlist.is_file() {
            return Err(format!("录播播放列表不存在: {}", playlist.display()));
        }
        let vod_playlist = create_vod_playlist(&playlist).await?;
        let audio_result = crate::ffmpeg::extract_full_audio(&vod_playlist).await;
        let _ = tokio::fs::remove_file(&vod_playlist).await;
        let audio_path = audio_result?;

        state
            .db
            .update_record_summary_stage(&platform, &room_id, &live_id, "transcribing")
            .await
            .map_err(String::from)?;
        reporter.update("生成字幕中").await;
        let generated = crate::ffmpeg::generate_video_subtitle(
            Some(&reporter),
            &audio_path,
            &config.subtitle_generator_type,
            &state.resource_dir,
            &config.whisper_model,
            &config.whisper_prompt,
            &config.openai_api_key,
            &config.openai_api_endpoint,
            &config.whisper_language,
        )
        .await;
        let _ = tokio::fs::remove_file(&audio_path).await;
        let generated = generated?;
        if generated.subtitle_content.is_empty() {
            return Err("未识别出有效语音内容".to_string());
        }
        let subtitle_srt = generated
            .subtitle_content
            .iter()
            .map(item_to_srt)
            .collect::<String>();
        let subtitle_text = subtitle_for_llm(&generated.subtitle_content);
        let duration = generated
            .subtitle_content
            .last()
            .map(|item| time_to_seconds(&item.end_time))
            .unwrap_or_default();
        state
            .db
            .save_record_summary_subtitle(
                &platform,
                &room_id,
                &live_id,
                &subtitle_srt,
                &subtitle_text,
                duration,
            )
            .await
            .map_err(String::from)?;
        (subtitle_text, subtitle_srt, duration)
    };

    state
        .db
        .update_record_summary_stage(&platform, &room_id, &live_id, "summarizing")
        .await
        .map_err(String::from)?;
    reporter.update("使用 LLM 总结直播内容中").await;
    let llm_config = state.config.read().await.llm.clone();
    let output = summarize_transcript(&llm_config, &subtitle_text, duration).await?;
    let markdown = summary_markdown(&output);
    let highlights_json = serde_json::to_string(&output.highlights).map_err(|e| e.to_string())?;
    state
        .db
        .complete_record_summary(
            &platform,
            &room_id,
            &live_id,
            &markdown,
            &highlights_json,
            &llm_config.provider,
            &llm_config.model,
        )
        .await
        .map_err(String::from)?;
    Ok(())
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_archive_summary(
    state: state_type!(),
    platform: String,
    room_id: String,
    live_id: String,
) -> Result<Option<RecordSummaryRow>, String> {
    Ok(state
        .db
        .get_record_summary(&platform, &room_id, &live_id)
        .await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn get_archive_summary_statuses(
    state: state_type!(),
) -> Result<Vec<RecordSummaryStatusRow>, String> {
    Ok(state.db.get_record_summary_statuses().await?)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn generate_archive_summary(
    state: state_type!(),
    platform: String,
    room_id: String,
    live_id: String,
    force: bool,
) -> Result<TaskRow, String> {
    let record = state.db.get_record(&room_id, &live_id).await?;
    if record.platform != platform {
        return Err("录播平台与请求参数不一致".to_string());
    }
    let platform_type = recorder::platforms::PlatformType::from_str(&platform)?;
    if let Some(recorder) = state
        .recorder_manager
        .get_recorder_info(platform_type, &room_id)
        .await
    {
        if recorder.recording && recorder.live_id == live_id {
            return Err("录播仍在进行中，请在直播结束后生成 Summary".to_string());
        }
    }
    let llm_config = state.config.read().await.llm.clone();
    if llm_config.model.trim().is_empty() {
        return Err("请先配置 Summary 使用的 LLM 模型".to_string());
    }
    if llm_config.provider == "openai" && llm_config.api_key.trim().is_empty() {
        return Err("请先配置 LLM API Key".to_string());
    }
    if let Some(summary) = state
        .db
        .get_record_summary(&platform, &room_id, &live_id)
        .await?
    {
        if summary.status == "processing" {
            if let Some(task_id) = summary.task_id {
                return Ok(state.db.get_task(&task_id).await?);
            }
        }
        if summary.status == "success" && !force {
            return Err("该录播已经生成 Summary".to_string());
        }
    }

    let task = state
        .db
        .generate_task(
            "generate_archive_summary",
            "",
            &serde_json::json!({
                "platform": platform,
                "room_id": room_id,
                "live_id": live_id,
                "force": force,
            })
            .to_string(),
        )
        .await?;
    state
        .db
        .start_record_summary(&platform, &room_id, &live_id, &task.id, force)
        .await?;

    #[cfg(feature = "gui")]
    let emitter = EventEmitter::new(state.app_handle.clone());
    #[cfg(feature = "headless")]
    let emitter = EventEmitter::new(state.progress_manager.get_event_sender());
    let reporter = ProgressReporter::new(state.db.clone(), &emitter, &task.id).await?;

    #[cfg(feature = "gui")]
    let state_clone = (*state).clone();
    #[cfg(feature = "headless")]
    let state_clone = state.clone();
    let task_id = task.id.clone();
    let task_platform = platform.clone();
    let task_room_id = room_id.clone();
    let task_live_id = live_id.clone();
    let add_result = state
        .task_manager
        .add_task(Task::new(
            task_id.clone(),
            TaskPriority::Normal,
            async move {
                match run_summary(
                    state_clone.clone(),
                    reporter.clone(),
                    task_platform.clone(),
                    task_room_id.clone(),
                    task_live_id.clone(),
                    force,
                )
                .await
                {
                    Ok(()) => {
                        reporter.finish(true, "Summary 生成完成").await;
                        state_clone
                            .db
                            .update_task(&task_id, "success", "Summary 生成完成", None)
                            .await
                            .map_err(String::from)?;
                        Ok(())
                    }
                    Err(error) => {
                        let _ = state_clone
                            .db
                            .fail_record_summary(
                                &task_platform,
                                &task_room_id,
                                &task_live_id,
                                &error,
                            )
                            .await;
                        let _ = state_clone
                            .db
                            .update_task(&task_id, "failed", &error, None)
                            .await;
                        reporter.finish(false, &error).await;
                        Err(error)
                    }
                }
            },
        ))
        .await;
    if let Err(error) = add_result {
        let _ = state
            .db
            .fail_record_summary(&platform, &room_id, &live_id, &error)
            .await;
        let _ = state.db.update_task(&task.id, "failed", &error, None).await;
        return Err(error);
    }
    Ok(task)
}

#[cfg_attr(feature = "gui", tauri::command)]
pub async fn delete_archive_summary(
    state: state_type!(),
    platform: String,
    room_id: String,
    live_id: String,
) -> Result<(), String> {
    state
        .db
        .delete_record_summary(&platform, &room_id, &live_id)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_inside_code_fence() {
        let output: SummaryOutput = parse_json_response(
            "```json\n{\"overview\":\"ok\",\"main_topics\":[],\"highlights\":[]}\n```",
        )
        .unwrap();
        assert_eq!(output.overview, "ok");
    }
}
