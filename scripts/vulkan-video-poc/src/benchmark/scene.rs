use native_overlay::{AssSource, RenderInput, SourceKind};
use std::path::Path;

#[path = "../scene.rs"]
mod poc;
pub use poc::BACKGROUND;

fn ass_time(seconds: f64) -> String {
    let cs = (seconds * 100.0).round() as u64;
    format!(
        "{}:{:02}:{:02}.{:02}",
        cs / 360_000,
        cs / 6000 % 60,
        cs / 100 % 60,
        cs % 100
    )
}

pub fn make(workload: &str, duration: f64, red: &Path) -> RenderInput {
    if workload == "poc" {
        return poc::scene(red, duration);
    }
    // Same ASS style as Danmu2AssOptions::default()/danmu_render_plan. Arrival
    // density is a stated synthetic workload, not a user default or a claim
    // about the application's normal-density scheduler.
    let alpha = ((1.0_f64 - 0.8) * 255.0) as u8;
    let mut text = format!(
        "[Script Info]\nTitle: Default-style benchmark\nScriptType: v4.00+\nCollisions: Normal\nPlayResX: 1280\nPlayResY: 720\nTimer: 10.0000\n\
         [V4+ Styles]\n\
         Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
         Style: Default,微软雅黑,36,&H{alpha:02X}FFFFFF,&H{alpha:02X}FFFFFF,&H{alpha:02X}000000,&H{alpha:02X}000000,-1,0,0,0,100,100,0,0,1,1,0,4,20,20,2,0\n\
         [Events]\n\
         Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"
    );
    let rate = if workload == "dense" { 10 } else { 2 };
    let rows = if workload == "dense" { 18 } else { 8 };
    for index in 0..(duration * f64::from(rate)).ceil() as usize {
        let start = index as f64 / f64::from(rate);
        let y = 28 + (index % rows) * 36;
        text.push_str(&format!(
            "Dialogue: 0,{},{},Default,,0,0,0,,{{\\move(1313,{y},-380,{y})}}默认样式测试 弹幕 {index:04} GPU CPU 🚀\n",
            ass_time(start), ass_time(start + 8.0),
        ));
    }
    RenderInput {
        documents: vec![AssSource {
            text,
            kind: SourceKind::Danmaku,
        }],
        images: Vec::new(),
    }
}
