//! AI-suggested covers for uploads: a vision model picks the cover frame and
//! the text style and placement for the title; the frontend composes the
//! picture over the returned frame with its canvas cover pipeline.

use base64::Engine;

use crate::agent::{llm_prompt, llm_prompt_with_images, LlmImage};
use crate::config::LlmConfig;
use crate::handlers::upload::{ensure_editable, frame_video_path};
use crate::handlers::utils::parse_json_response;
use crate::state::State;
use crate::state_type;

#[cfg(feature = "gui")]
use tauri::State as TauriState;

// The style ids in the prompt are the ones `TEXT_STYLES` in
// `src/lib/cover-elements.ts` can draw. Keep the two lists in sync.
const COVER_PREAMBLE: &str = r#"
你是 Bilibili 视频封面设计师。用户会给出视频标题、简介和标签，并按编号 0 到 N-1 依次附上候选画面帧。
请完成三件事：
1. 选出最适合做封面的一帧：画面清晰、有代表性、信息量足、构图适合叠加文字，避免大面积纯色、过暗或模糊的画面。
2. 为标题大字从这些样式中选择对比度和可读性最好的一个：yellow、white、red、orange、pink、blue、tag-yellow、tag-red、tag-black、shadow。深色或杂乱画面优先选亮色描边样式（如 yellow），浅色画面可选带底色样式（如 tag-yellow）。
3. 给出文字中心的摆放位置：x、y 分别是封面宽和高的比例（0 到 1），避开画面主体，通常放在下方居中。
输出必须是合法 JSON，不要使用 Markdown 代码块，不要输出 JSON 之外的任何文字。
返回结构：{"frame_index":0,"style":"yellow","position":{"x":0.5,"y":0.78}}
"#;

/// Candidate frames spread over the body of the recording, skipping the black
/// head most recordings start with.
const CANDIDATE_FRAMES: usize = 6;

/// Width of the frames sent to the model, to keep the request small.
const CANDIDATE_WIDTH: u32 = 512;
/// Width of the frame returned for the frontend to compose over; the canvas
/// pipeline resamples further as needed.
const COVER_WIDTH: u32 = 1920;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct CoverPosition {
    pub x: f64,
    pub y: f64,
}

/// What the model returns for one cover.
#[derive(Debug, serde::Deserialize)]
struct FrameSuggestion {
    #[serde(default)]
    frame_index: usize,
    #[serde(default = "default_style")]
    style: String,
    #[serde(default = "default_position")]
    position: CoverPosition,
}

impl Default for FrameSuggestion {
    fn default() -> Self {
        Self {
            frame_index: 0,
            style: default_style(),
            position: default_position(),
        }
    }
}

fn default_style() -> String {
    "yellow".to_string()
}

fn default_position() -> CoverPosition {
    CoverPosition { x: 0.5, y: 0.78 }
}

/// The cover suggestion the frontend composes over.
#[derive(Debug, serde::Serialize)]
pub struct CoverSuggestion {
    /// Where the chosen frame came from, for reference.
    pub timestamp: f64,
    /// The chosen frame as base64 JPEG.
    pub frame_base64: String,
    pub style: String,
    pub position: CoverPosition,
    /// Set when the vision call failed and defaults were used instead.
    pub warning: Option<String>,
}

fn validate_llm_config(config: &LlmConfig) -> Result<(), String> {
    if config.model.trim().is_empty() {
        return Err("请先在设置中配置 AI 模型：未选择模型".to_string());
    }
    if config.provider == "openai" && config.api_key.trim().is_empty() {
        return Err("请先在设置中配置 AI 模型：未配置 API Key".to_string());
    }
    Ok(())
}

async fn request_frame_suggestion(
    config: &LlmConfig,
    prompt: &str,
    images: &[LlmImage],
) -> Result<FrameSuggestion, String> {
    let first = llm_prompt_with_images(config, COVER_PREAMBLE, prompt, images).await?;
    match parse_json_response::<FrameSuggestion>(&first) {
        Ok(suggestion) => Ok(suggestion),
        Err(first_error) => {
            // Repairing the shape needs no images again.
            let repair_prompt = format!(
                "请把下面的响应修复成指定结构的合法 JSON。不得增加原响应中没有的内容。\n\n错误：{first_error}\n\n响应：\n{first}\n\n结构：{{\"frame_index\":0,\"style\":\"yellow\",\"position\":{{\"x\":0.5,\"y\":0.78}}}}"
            );
            let repaired = llm_prompt(config, COVER_PREAMBLE, &repair_prompt).await?;
            parse_json_response(&repaired)
        }
    }
}

/// Suggest an AI cover for the upload `id`: candidate frames go to the vision
/// model, which picks one plus the title's style and position. A failed or
/// unparsable answer falls back to a default frame and layout, reported in
/// `warning`; the frontend composes the title over the returned frame.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn suggest_upload_cover(
    state: state_type!(),
    id: i64,
) -> Result<CoverSuggestion, String> {
    let upload = state.db.get_upload(id).await?;
    ensure_editable(&upload)?;
    let video = frame_video_path(&state, &upload)
        .await
        .ok_or_else(|| "没有可用于生成封面的视频画面".to_string())?;

    let config = state.config.read().await.llm.clone();
    validate_llm_config(&config)?;

    let metadata = ffmpeg_utils::extract_video_metadata(&video)
        .await
        .map_err(|e| format!("无法读取视频信息：{e}"))?;
    let duration = metadata.duration;
    if !duration.is_finite() || duration <= 0.0 {
        return Err("无法确定视频时长".to_string());
    }

    let timestamps: Vec<f64> = (0..CANDIDATE_FRAMES)
        .map(|i| duration * (0.1 + 0.8 * (i as f64 / (CANDIDATE_FRAMES - 1) as f64)))
        .collect();
    let mut candidates: Vec<(usize, String)> = Vec::new();
    for (index, timestamp) in timestamps.iter().enumerate() {
        match ffmpeg_utils::extract_jpeg_scaled(&video, *timestamp, Some(7), Some(CANDIDATE_WIDTH))
            .await
        {
            Ok(data) => candidates.push((
                index,
                base64::engine::general_purpose::STANDARD.encode(&data),
            )),
            Err(error) => log::warn!("候选帧 {index} 抽取失败：{error}"),
        }
    }
    if candidates.is_empty() {
        return Err("无法从视频中抽取画面帧".to_string());
    }

    let profile: serde_json::Value =
        serde_json::from_str(&upload.profile).unwrap_or(serde_json::Value::Null);
    let field = |name: &str| {
        profile[name]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let prompt = format!(
        "视频标题：{}\n视频简介：{}\n视频标签：{}\n\n下面按编号 0 到 {} 依次附上候选画面帧，请选出封面画面帧，并给出标题大字的样式与摆放位置。",
        field("title"),
        field("desc"),
        field("tag"),
        candidates.len() - 1,
    );
    let images: Vec<LlmImage> = candidates
        .iter()
        .map(|(_, data)| LlmImage {
            mime_type: "image/jpeg".to_string(),
            data: data.clone(),
        })
        .collect();

    let (suggestion, warning) = match request_frame_suggestion(&config, &prompt, &images).await {
        Ok(suggestion) => (suggestion, None),
        Err(error) => {
            log::warn!("AI 封面建议失败，使用默认画面：{error}");
            (FrameSuggestion::default(), Some(error))
        }
    };

    // `frame_index` counts the candidate slots, some of which may have failed
    // to extract: take the nearest still-available frame. Without an AI pick,
    // a middle candidate stands in, close to the usual default-cover rule.
    let slot = if warning.is_some() {
        candidates.len() / 2
    } else {
        candidates
            .iter()
            .position(|(index, _)| *index == suggestion.frame_index.min(CANDIDATE_FRAMES - 1))
            .unwrap_or(0)
    };
    let timestamp = timestamps[candidates[slot].0];

    let frame = ffmpeg_utils::extract_jpeg_scaled(&video, timestamp, Some(2), Some(COVER_WIDTH))
        .await
        .map_err(|e| format!("封面帧抽取失败：{e}"))?;

    Ok(CoverSuggestion {
        timestamp,
        frame_base64: base64::engine::general_purpose::STANDARD.encode(&frame),
        style: suggestion.style,
        position: suggestion.position,
        warning,
    })
}
