use native_overlay::{AssSource, ImageEvent, RenderInput, SourceKind};
use std::path::Path;

pub const BACKGROUND: [u8; 4] = [24, 48, 96, 255];

/// Use the production ASS/emoji/image scene implementation, not a replacement
/// Vulkan triangle renderer. Asset creation happens before GPU frame rendering.
pub fn scene(red: &Path, duration: f64) -> RenderInput {
    let start = duration * 0.25;
    let end = duration * 0.75;
    let time = |value: f64| {
        let centiseconds = (value * 100.0).round() as u64;
        format!(
            "{}:{:02}:{:02}.{:02}",
            centiseconds / 360_000,
            centiseconds / 6_000 % 60,
            centiseconds / 100 % 60,
            centiseconds % 100
        )
    };
    let text = format!(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: 1920\nPlayResY: 1080\n\
         [V4+ Styles]\n\
         Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
         Style: Default,Arial,80,&H00FFFFFF,&H00FFFFFF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,0,5,0,0,0,1\n\
         [Events]\n\
         Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
         Dialogue: 0,{},{},Default,,0,0,0,,GPU Vulkan 中文 🚀\n",
        time(start),
        time(end),
    );
    RenderInput {
        documents: vec![AssSource {
            text,
            kind: SourceKind::Subtitle,
        }],
        images: vec![ImageEvent {
            path: red.to_path_buf(),
            start: 0.0,
            end: duration,
            from: (240.0, 160.0),
            to: (1440.0, 160.0),
            width: 160.0,
            height: 160.0,
            opacity: 0.75,
        }],
    }
}
