use base64::{engine::general_purpose::STANDARD, Engine};
use native_overlay::{AssSource, RenderInput, SourceKind};

use crate::ffmpeg::native_overlay::system_renderer;

const MAX_DOCUMENT_BYTES: usize = 256 * 1024;

fn validate_preview(ass_content: &str, time: f64, font_name: &str) -> Result<(), String> {
    if ass_content.is_empty() || ass_content.len() > MAX_DOCUMENT_BYTES {
        return Err("字幕预览文档为空或超过 256 KiB 上限".to_string());
    }
    if !time.is_finite() || time < 0.0 {
        return Err("字幕预览时间必须为有限的非负数".to_string());
    }
    if font_name.len() > 512 || font_name.contains('\0') {
        return Err("字幕字体名称无效".to_string());
    }
    Ok(())
}

/// Only app-generated, active-event ASS documents belong here.
/// Style/font fields in the document remain
/// authoritative; font_name is not interpreted as a filesystem path.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn render_generated_subtitle_preview(
    ass_content: String,
    time: f64,
    font_name: String,
) -> Result<String, String> {
    validate_preview(&ass_content, time, &font_name)?;
    tokio::task::spawn_blocking(move || {
        // Construct and drop the renderer inside this closure: native shaping
        // and graphics handles need not be Send. Burns create theirs the same
        // way, so the preview shows the fonts they draw with.
        let input = RenderInput {
            documents: vec![AssSource {
                text: ass_content,
                kind: SourceKind::Subtitle,
            }],
            ..Default::default()
        };
        let mut renderer = system_renderer(1920, 1080, input)?;
        let png = renderer.png(time)?;
        Ok(format!("data:image/png;base64,{}", STANDARD.encode(png)))
    })
    .await
    .map_err(|error| format!("字幕预览渲染任务失败：{error}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_rejects_unbounded_or_invalid_inputs_before_rendering() {
        assert!(validate_preview("generated", 1.0, "Arial").is_ok());
        assert!(validate_preview("", 1.0, "Arial").is_err());
        assert!(validate_preview(&"x".repeat(MAX_DOCUMENT_BYTES + 1), 1.0, "Arial").is_err());
        // Bound UTF-8 bytes, not character count.
        assert!(validate_preview(&"字".repeat(MAX_DOCUMENT_BYTES / 3 + 1), 1.0, "Arial").is_err());
        for time in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.01] {
            assert!(validate_preview("generated", time, "Arial").is_err());
        }
        assert!(validate_preview("generated", 0.0, "bad\0font").is_err());
        assert!(validate_preview("generated", 0.0, &"x".repeat(513)).is_err());
    }
}
