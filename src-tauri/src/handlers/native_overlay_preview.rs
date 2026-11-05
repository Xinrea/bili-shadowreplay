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

fn preview_dimensions(width: Option<u32>, height: Option<u32>) -> Result<(u32, u32), String> {
    let width = width.unwrap_or(1920);
    let height = height.unwrap_or(1080);
    // Bound untrusted HTTP/IPC requests before allocating an RGBA surface.
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > 33_554_432
    {
        return Err("字幕预览分辨率无效或超出上限".to_string());
    }
    Ok((width, height))
}

/// Only app-generated, active-event ASS documents belong here.
/// Style/font fields in the document remain
/// authoritative; font_name is not interpreted as a filesystem path.
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn render_generated_subtitle_preview(
    ass_content: String,
    time: f64,
    font_name: String,
    width: Option<u32>,
    height: Option<u32>,
) -> Result<String, String> {
    validate_preview(&ass_content, time, &font_name)?;
    let (width, height) = preview_dimensions(width, height)?;
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
        let mut renderer = system_renderer(width, height, input)?;
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
    fn preview_accepts_portrait_and_4k_but_bounds_surface_allocation() {
        assert_eq!(preview_dimensions(None, None).unwrap(), (1920, 1080));
        assert_eq!(
            preview_dimensions(Some(1080), Some(1920)).unwrap(),
            (1080, 1920)
        );
        assert_eq!(
            preview_dimensions(Some(3840), Some(2160)).unwrap(),
            (3840, 2160)
        );
        assert!(preview_dimensions(Some(0), Some(1080)).is_err());
        assert!(preview_dimensions(Some(8192), Some(8192)).is_err());
        assert!(preview_dimensions(Some(u32::MAX), Some(1)).is_err());
    }

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
