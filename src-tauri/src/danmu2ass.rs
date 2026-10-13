use recorder::danmu::DanmuEntry;
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, path::PathBuf};

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

pub const PLAY_RES_X: f64 = 1280.0;
pub const PLAY_RES_Y: f64 = 720.0;
const BOTTOM_RESERVED: f64 = 50.0;
pub const R2L_TIME: f64 = 8.0;
const MAX_DELAY: f64 = 6.0;

#[derive(Deserialize, Serialize, Clone)]
pub struct Danmu2AssOptions {
    pub font_size: f64,
    pub opacity: f64, // 透明度，范围 0.0-1.0，0.0为完全透明，1.0为完全不透明
}

impl Default for Danmu2AssOptions {
    fn default() -> Self {
        Self {
            font_size: 36.0,
            opacity: 0.8, // 默认80%透明度
        }
    }
}

/// A local, decoded image with its footprint in ASS play-resolution units.
#[derive(Debug, Clone)]
pub struct DanmuImage {
    pub path: PathBuf,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone)]
pub struct ImageDanmu {
    pub image: DanmuImage,
    pub start: f64,
    pub top: f64,
}

pub struct DanmuRenderPlan {
    pub ass: String,
    pub images: Vec<ImageDanmu>,
}

/// Portable ASS exports remain text-only; image extensions such as VSFilterMod's
/// `\\1img` are not supported by libass or most ASS players.
pub fn danmu_to_ass(danmus: Vec<DanmuEntry>, options: Danmu2AssOptions) -> String {
    danmu_render_plan(&danmus, &options, |_| None).ass
}

/// Text and images must reserve space in the same scheduler. Resolve images
/// before scheduling so a failed download uses the full text footprint instead.
pub fn danmu_render_plan(
    danmus: &[DanmuEntry],
    options: &Danmu2AssOptions,
    resolve_image: impl Fn(&DanmuEntry) -> Option<DanmuImage>,
) -> DanmuRenderPlan {
    // Exclude SC and other preview-only events. Sort after clip/transition time
    // remapping, which can interleave messages from adjacent source ranges.
    let mut danmus: Vec<_> = danmus
        .iter()
        .filter(|danmu| danmu.is_danmu() && danmu.ts >= 0)
        .collect();
    danmus.sort_by_key(|danmu| danmu.ts);

    let font_size = options.font_size;
    let opacity = options.opacity;

    // 将透明度转换为十六进制Alpha值 (0.0-1.0 -> 0x00-0xFF)
    let alpha = ((1.0 - opacity) * 255.0) as u8;
    let alpha_hex = format!("{:02X}", alpha);

    // ASS header
    let header = format!(
        r"[Script Info]
Title: Bilibili Danmaku
ScriptType: v4.00+
Collisions: Normal
PlayResX: 1280
PlayResY: 720
Timer: 10.0000

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,微软雅黑,{},&H{}FFFFFF,&H{}FFFFFF,&H{}000000,&H{}000000,0,0,0,0,100,100,0,0,1,1,0,2,20,20,2,0

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
",
        font_size, alpha_hex, alpha_hex, alpha_hex, alpha_hex
    );

    let mut normal = normal_danmaku();
    let mut events = Vec::new();
    let mut images = Vec::new();
    for danmu in danmus {
        let image = resolve_image(danmu);
        let (width, height) = image
            .as_ref()
            .map(|image| (image.width, image.height))
            .unwrap_or_else(|| (danmu.content.len() as f64 * font_size * 0.6, font_size));
        let Some(pos) = normal(danmu.ts as f64 / 1000.0, width, height, false) else {
            continue;
        };
        if let Some(image) = image {
            images.push(ImageDanmu {
                image,
                start: pos.time,
                top: pos.top,
            });
        } else {
            // ASS alignment 2 uses the bottom-center anchor. Its left edge
            // travels from PLAY_RES_X to -width, matching the collision model
            // and the image overlay's motion exactly.
            events.push(format!(
                "Dialogue: 0,{},{},Default,,0,0,0,,{{\\move({},{},{},{})}}{}",
                format_time(pos.time),
                format_time(pos.time + R2L_TIME),
                PLAY_RES_X + width / 2.0,
                pos.top + height,
                -width / 2.0,
                pos.top + height,
                escape_text(&danmu.content)
            ));
        }
    }

    DanmuRenderPlan {
        ass: format!("{header}\n{}", events.join("\n")),
        images,
    }
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
        suggestions.sort_by(|a, b| a.p.total_cmp(&b.p));

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
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, s)| s)?;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(ts: i64, content: &str) -> DanmuEntry {
        DanmuEntry {
            ts,
            event_type: "danmu".into(),
            content: content.into(),
            emoji_content: None,
            user_name: None,
            price: None,
            sc_duration: None,
        }
    }

    fn image() -> DanmuImage {
        DanmuImage {
            path: "emoji.png".into(),
            width: 108.0,
            height: 54.0,
        }
    }

    #[test]
    fn images_replace_text_and_share_lanes_with_text() {
        let entries = vec![entry(1000, "[emoji]"), entry(1000, "hello")];
        let plan = danmu_render_plan(&entries, &Danmu2AssOptions::default(), |entry| {
            (entry.content == "[emoji]").then(image)
        });
        assert_eq!(plan.images.len(), 1);
        assert_eq!(plan.images[0].start, 1.0);
        assert_eq!(plan.images[0].top, 0.0);
        assert!(!plan.ass.contains("[emoji]"));
        // Text sits below the 54-unit-high image, not in a 36-unit text lane.
        assert!(plan.ass.contains("\\move(1334,90,-54,90)"));
        assert!(plan.ass.contains("hello"));
    }

    #[test]
    fn failed_images_fall_back_to_normal_text_layout() {
        let entries = vec![entry(1000, "[emoji]"), entry(1000, "hello")];
        let plan = danmu_render_plan(&entries, &Danmu2AssOptions::default(), |_| None);
        assert!(plan.images.is_empty());
        assert!(plan.ass.contains("[emoji]"));
        assert!(plan.ass.contains("\\move(1334,72,-54,72)}hello"));
    }

    #[test]
    fn portable_ass_export_keeps_emoji_text() {
        let mut danmu = entry(1000, "[emoji]");
        danmu.emoji_content = Some(danmu_stream::EmojiContent {
            url: "https://i0.hdslb.com/emoji.png".into(),
            width: Some(80),
            height: Some(80),
            emoticon_unique: None,
        });
        let ass = danmu_to_ass(vec![danmu], Danmu2AssOptions::default());
        assert!(ass.contains("[emoji]"));
        assert!(!ass.contains("\\1img"));
    }

    #[test]
    fn render_plan_orders_remapped_times_and_excludes_non_danmu() {
        let mut sc = entry(0, "super chat");
        sc.event_type = "super_chat".into();
        let entries = vec![
            entry(5000, "late"),
            sc,
            entry(-1, "before"),
            entry(1000, "early"),
        ];
        let plan = danmu_render_plan(&entries, &Danmu2AssOptions::default(), |_| Some(image()));
        assert_eq!(plan.images.len(), 2);
        assert_eq!(plan.images[0].start, 1.0);
        assert_eq!(plan.images[1].start, 5.0);
    }

    #[test]
    fn test_format_time_zero() {
        assert_eq!(format_time(0.0), "0:00:00.00");
    }

    #[test]
    fn test_format_time_seconds() {
        assert_eq!(format_time(5.5), "0:00:05.50");
    }

    #[test]
    fn test_format_time_minutes() {
        assert_eq!(format_time(65.25), "0:01:05.25");
    }

    #[test]
    fn test_format_time_hours() {
        assert_eq!(format_time(3661.99), "1:01:01.99");
    }

    #[test]
    fn test_escape_text_backslash() {
        assert_eq!(escape_text("a\\b"), "a\\\\b");
    }

    #[test]
    fn test_escape_text_braces() {
        assert_eq!(escape_text("{test}"), "｛test｝");
    }

    #[test]
    fn test_escape_text_carriage_return() {
        // \r is removed, \n becomes \N (ASS line break)
        assert_eq!(escape_text("line\r\n"), "line\\N");
    }

    #[test]
    fn test_escape_text_plain() {
        assert_eq!(escape_text("hello world"), "hello world");
    }

    #[test]
    fn test_danmu2ass_options_default() {
        let opts = Danmu2AssOptions::default();
        assert_eq!(opts.font_size, 36.0);
        assert_eq!(opts.opacity, 0.8);
    }

    #[test]
    fn test_danmu_to_ass_empty() {
        let result = danmu_to_ass(vec![], Danmu2AssOptions::default());
        assert!(result.contains("[Script Info]"));
        assert!(result.contains("[V4+ Styles]"));
        assert!(result.contains("[Events]"));
    }

    #[test]
    fn test_danmu_to_ass_with_entries() {
        let danmus = vec![
            DanmuEntry {
                ts: 1000,
                event_type: "danmu".to_string(),
                content: "hello".to_string(),
                emoji_content: None,
                user_name: None,
                price: None,
                sc_duration: None,
            },
            DanmuEntry {
                ts: 5000,
                event_type: "danmu".to_string(),
                content: "world".to_string(),
                emoji_content: None,
                user_name: None,
                price: None,
                sc_duration: None,
            },
        ];
        let result = danmu_to_ass(danmus, Danmu2AssOptions::default());
        assert!(result.contains("[Events]"));
        assert!(result.contains("hello"));
        assert!(result.contains("world"));
    }

    #[test]
    fn test_danmu_to_ass_skips_super_chats() {
        let danmus = vec![
            DanmuEntry {
                ts: 1000,
                event_type: "danmu".to_string(),
                content: "hello".to_string(),
                emoji_content: None,
                user_name: None,
                price: None,
                sc_duration: None,
            },
            DanmuEntry {
                ts: 5000,
                event_type: "super_chat".to_string(),
                content: "super chat".to_string(),
                emoji_content: None,
                user_name: Some("alice".to_string()),
                price: Some(30),
                sc_duration: Some(60),
            },
        ];
        let result = danmu_to_ass(danmus, Danmu2AssOptions::default());
        assert!(result.contains("hello"));
        assert!(!result.contains("super chat"));
    }

    #[test]
    fn test_danmu_to_ass_opacity() {
        // opacity 1.0 -> alpha 0x00
        let result = danmu_to_ass(
            vec![],
            Danmu2AssOptions {
                font_size: 36.0,
                opacity: 1.0,
            },
        );
        assert!(result.contains("&H00FFFFFF"));

        // opacity 0.0 -> alpha 0xFF
        let result = danmu_to_ass(
            vec![],
            Danmu2AssOptions {
                font_size: 36.0,
                opacity: 0.0,
            },
        );
        assert!(result.contains("&HFFFFFFFF"));
    }

    #[test]
    fn test_danmu_to_ass_font_size() {
        let result = danmu_to_ass(
            vec![],
            Danmu2AssOptions {
                font_size: 48.0,
                opacity: 0.8,
            },
        );
        assert!(result.contains(",48,"));
    }
}
