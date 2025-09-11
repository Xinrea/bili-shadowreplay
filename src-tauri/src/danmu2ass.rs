use crate::recorder::danmu::DanmuEntry;
use std::collections::VecDeque;

// code reference: https://github.com/tiansh/us-danmaku/blob/master/bilibili/bilibili_ASS_Danmaku_Downloader.user.js

#[derive(Debug, Clone)]
struct UsedSpace {
    p: f64,  // top position
    m: f64,  // bottom position
    tf: f64, // time when fully visible
    td: f64, // time when completely gone
    b: bool, // is bottom reserved
}

#[derive(Debug, Clone)]
struct PositionSuggestion {
    p: f64, // position
    r: f64, // delay
}

#[derive(Debug, Clone)]
struct DanmakuPosition {
    top: f64,
    time: f64,
}

const PLAY_RES_X: f64 = 1280.0;
const PLAY_RES_Y: f64 = 720.0;
const BOTTOM_RESERVED: f64 = 50.0;
const R2L_TIME: f64 = 8.0;
const MAX_DELAY: f64 = 6.0;

pub fn danmu_to_ass(danmus: Vec<DanmuEntry>) -> String {
    // ASS header
    let header = r"[Script Info]
Title: Bilibili Danmaku
ScriptType: v4.00+
Collisions: Normal
PlayResX: 1280
PlayResY: 720
Timer: 10.0000

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,微软雅黑,36,&H7fFFFFFF,&H7fFFFFFF,&H7f000000,&H7f000000,0,0,0,0,100,100,0,0,1,1,0,2,20,20,2,0

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
";

    let mut normal = normal_danmaku();
    let font_size = 36.0; // Default font size

    // Convert danmus to ASS events
    let events = danmus
        .iter()
        .filter_map(|danmu| {
            // Calculate text width (approximate)
            let text_width = danmu.content.len() as f64 * font_size * 0.6;

            // Convert timestamp from ms to seconds
            let t0s = danmu.ts as f64 / 1000.0;

            // Get position from normal_danmaku
            let pos = normal(t0s, text_width, font_size, false)?;

            // Convert timestamp to ASS time format (H:MM:SS.CC)
            let start_time = format_time(pos.time);
            let end_time = format_time(pos.time + R2L_TIME);

            // Escape special characters in the text
            let text = escape_text(&danmu.content);

            // Create ASS event line with movement effect
            Some(format!(
                "Dialogue: 0,{},{},Default,,0,0,0,,{{\\move({},{},{},{})}}{}",
                start_time,
                end_time,
                PLAY_RES_X + text_width / 2.0,
                pos.top + font_size, // Start position
                -text_width,
                pos.top + font_size, // End position
                text
            ))
        })
        .collect::<Vec<String>>()
        .join("\n");

    // Combine header and events
    format!("{header}\n{events}")
}

fn format_time(seconds: f64) -> String {
    let hours = (seconds / 3600.0) as i32;
    let minutes = ((seconds % 3600.0) / 60.0) as i32;
    let seconds = seconds % 60.0;
    format!("{hours}:{minutes:02}:{seconds:05.2}")
}

fn escape_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('{', "｛")
        .replace('}', "｝")
        .replace('\r', "")
        .replace('\n', "\\N")
}

fn normal_danmaku() -> impl FnMut(f64, f64, f64, bool) -> Option<DanmakuPosition> {
    let mut used = VecDeque::new();
    used.push_back(UsedSpace {
        p: f64::NEG_INFINITY,
        m: 0.0,
        tf: f64::INFINITY,
        td: f64::INFINITY,
        b: false,
    });
    used.push_back(UsedSpace {
        p: PLAY_RES_Y,
        m: f64::INFINITY,
        tf: f64::INFINITY,
        td: f64::INFINITY,
        b: false,
    });
    used.push_back(UsedSpace {
        p: PLAY_RES_Y - BOTTOM_RESERVED,
        m: PLAY_RES_Y,
        tf: f64::INFINITY,
        td: f64::INFINITY,
        b: true,
    });

    move |t0s: f64, wv: f64, hv: f64, b: bool| {
        let t0l = (PLAY_RES_X / (wv + PLAY_RES_X)) * R2L_TIME + t0s;

        // Synchronize used spaces
        used.retain(|space| space.tf > t0s || space.td > t0l);

        // Find available positions
        let mut suggestions = Vec::new();
        for space in &used {
            if space.m > PLAY_RES_Y {
                continue;
            }

            let p = space.m;
            let m = p + hv;
            let mut time_actual_start = t0s;
            let mut time_actual_leave = t0l;

            for other in &used {
                if other.p >= m || other.m <= p {
                    continue;
                }
                if other.b && b {
                    continue;
                }
                time_actual_start = time_actual_start.max(other.tf);
                time_actual_leave = time_actual_leave.max(other.td);
            }

            suggestions.push(PositionSuggestion {
                p,
                r: (time_actual_start - t0s).max(time_actual_leave - t0l),
            });
        }

        // Sort suggestions by position
        suggestions.sort_by(|a, b| a.p.partial_cmp(&b.p).unwrap());

        // Filter out suggestions with too much delay
        let mut mr = MAX_DELAY;
        suggestions.retain(|s| {
            if s.r >= mr {
                false
            } else {
                mr = s.r;
                true
            }
        });

        if suggestions.is_empty() {
            return None;
        }

        // Score and select best position
        let best = suggestions
            .iter()
            .map(|s| {
                let score = if s.r > MAX_DELAY {
                    f64::NEG_INFINITY
                } else {
                    1.0 - ((s.r / MAX_DELAY).powi(2) + (s.p / PLAY_RES_Y).powi(2)).sqrt()
                        * std::f64::consts::FRAC_1_SQRT_2
                };
                (score, s)
            })
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap())
            .unwrap()
            .1;

        let ts = t0s + best.r;
        let tf = (wv / (wv + PLAY_RES_X)) * R2L_TIME + ts;
        let td = R2L_TIME + ts;

        used.push_back(UsedSpace {
            p: best.p,
            m: best.p + hv,
            tf,
            td,
            b: false,
        });

        Some(DanmakuPosition {
            top: best.p,
            time: ts,
        })
    }
}
