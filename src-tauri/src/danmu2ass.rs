use recorder::danmu::DanmuEntry;
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, path::PathBuf};

// The layout follows Bilibili's live-room danmaku engine
// (https://s1.hdslb.com/bfs/live-pkg/danmaku/danmaku-v2.js: the roll mode,
// its space manager and `getBaseDmDuration`; the room's render queue), with
// the room player's default settings: the whole screen, normal density and
// speed, no top offset. The video is laid out as a full-screen 1080p player,
// in its CSS pixels.

pub const PLAY_RES_Y: f64 = 720.0;
const CONTAINER_WIDTH: f64 = 1920.0;
const CONTAINER_HEIGHT: f64 = 1080.0;
/// Play-resolution units per container pixel.
const PX: f64 = PLAY_RES_Y / CONTAINER_HEIGHT;
/// The em of 微软雅黑 within the line box an ASS font size sets (its
/// unitsPerEm over winAscent + winDescent). Common CJK fallbacks (PingFang,
/// Noto Sans CJK) have a smaller em, so glyphs never outgrow the layout.
const FONT_EM: f64 = 2048.0 / 2703.0;
/// Row height, and image height, in ems.
const LINE_HEIGHT: f64 = 1.125;
/// Messages enter this far beyond the right edge, and leave this far beyond
/// the left one.
const SAFE_RIGHT: f64 = 50.0;
const SAFE_LEFT: f64 = 50.0;
/// The left edge of a message as it enters.
const ENTRY_X: f64 = CONTAINER_WIDTH + 1.0 + SAFE_RIGHT;
/// Seconds a message must stay behind the one ahead of it in its row, at
/// normal density.
const FOLLOW_GAP: f64 = 0.1;
/// The room draws arriving messages through at most this many timers, each
/// set this much later per timer already waiting...
const MAX_TIMERS: usize = 10;
const TIMER_STEP: f64 = 0.1;
/// ...and keeps at most one more than this many waiting, dropping the oldest.
const QUEUE_LIMIT: usize = 20;

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

/// An image crossing the screen: its left edge moves from `from` to `to`
/// between `start` and `end`, in play-resolution units and seconds.
#[derive(Debug, Clone)]
pub struct ImageDanmu {
    pub image: DanmuImage,
    pub start: f64,
    pub end: f64,
    pub top: f64,
    pub from: f64,
    pub to: f64,
}

pub struct DanmuRenderPlan {
    pub ass: String,
    pub images: Vec<ImageDanmu>,
}

/// The height of image danmaku, in play-resolution units: a row, as Bilibili
/// draws emoticons.
pub fn image_height(font_size: f64) -> f64 {
    LINE_HEIGHT * font_size * FONT_EM
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

    // ASS header. Text is bold, as the room draws it by default, and anchored
    // by the middle of its left edge, which is what the layout positions; its
    // width is only estimated.
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
Style: Default,微软雅黑,{},&H{}FFFFFF,&H{}FFFFFF,&H{}000000,&H{}000000,-1,0,0,0,100,100,0,0,1,1,0,4,20,20,2,0

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
",
        font_size, alpha_hex, alpha_hex, alpha_hex, alpha_hex
    );

    let em = font_size * FONT_EM / PX;
    let line_height = (LINE_HEIGHT * em).ceil();
    let mut space = Space::default();
    let mut events = Vec::new();
    let mut images = Vec::new();
    let arrivals = danmus
        .into_iter()
        .map(|danmu| (danmu.ts as f64 / 1000.0, danmu))
        .collect();
    for (now, danmu) in pace(arrivals) {
        let text = danmu.content.trim();
        let image = resolve_image(danmu);
        let (width, height) = image
            .as_ref()
            .map(|image| (image.width / PX, image.height / PX))
            .unwrap_or_else(|| (text_width(text, em), line_height));
        let flight = Flight::new(now, width);
        let Some(top) = space.place(now, &flight, height) else {
            continue;
        };
        let (start, end) = (now, now + flight.duration);
        let (from, to) = (ENTRY_X * PX, (ENTRY_X - flight.travel) * PX);
        if let Some(image) = image {
            images.push(ImageDanmu {
                image,
                start,
                end,
                top: top * PX,
                from,
                to,
            });
        } else {
            let y = coordinate((top + height / 2.0) * PX);
            events.push(format!(
                "Dialogue: 0,{},{},Default,,0,0,0,,{{\\move({},{y},{},{y})}}{}",
                format_time(start),
                format_time(end),
                coordinate(from),
                coordinate(to),
                escape_text(text)
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

fn coordinate(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn escape_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('{', "｛")
        .replace('}', "｝")
        .replace('\r', "")
        .replace('\n', "\\N")
}

/// The room player doesn't draw a message the moment it arrives. It queues
/// it, and a timer draws it later by `TIMER_STEP` for each timer already
/// waiting: a burst is spread out to about ten messages a second, and its
/// oldest messages are dropped while more keep arriving. A quiet stream is
/// drawn at once. Returns when each message kept is drawn, in order.
fn pace<T>(arrivals: Vec<(f64, T)>) -> Vec<(f64, T)> {
    let mut arrivals = arrivals.into_iter().peekable();
    let mut queue = VecDeque::new();
    let mut timers: Vec<(f64, T)> = Vec::new();
    let mut drawn = Vec::new();
    loop {
        let timer = timers
            .iter()
            .enumerate()
            .min_by(|a, b| a.1 .0.total_cmp(&b.1 .0))
            .map(|(index, (at, _))| (index, *at));
        let arrival = arrivals.peek().map(|(time, _)| *time);
        // Messages arriving together are all queued before a timer fires.
        let now = match (timer, arrival) {
            (None, None) => break,
            (Some((index, at)), arrival) if arrival.is_none_or(|time| at < time) => {
                drawn.push(timers.swap_remove(index));
                at
            }
            _ => {
                let Some((time, item)) = arrivals.next() else {
                    break;
                };
                if queue.len() > QUEUE_LIMIT {
                    queue.pop_front();
                }
                queue.push_back(item);
                time
            }
        };
        while timers.len() < MAX_TIMERS {
            let Some(item) = queue.pop_front() else {
                break;
            };
            timers.push((now + timers.len() as f64 * TIMER_STEP, item));
        }
    }
    drawn
}

/// Bilibili's width of `text` in pixels at `em`. Common characters are
/// measured from a table, the rest are rendered, which this estimates with
/// 微软雅黑's widths.
fn text_width(text: &str, em: f64) -> f64 {
    // Bilibili's table for ' '..='@', relative to '9', which is 0.6.
    const PUNCTUATION: [f64; 33] = [
        0.5, 0.4, 0.55, 0.6, 0.6, 1.0, 0.8, 0.36, 0.4, 0.4, 0.6, 0.7, 0.4, 0.7, 0.4, 0.6, 0.65,
        0.5, 0.6, 0.6, 0.6, 0.6, 0.6, 0.6, 0.6, 0.6, 0.4, 0.4, 0.7, 0.7, 0.7, 0.6, 0.95,
    ];
    // The width of '9' in 微软雅黑, and of the rest of ASCII on average.
    const DIGIT: f64 = 0.55;
    const HALF: f64 = 0.55;
    let mut width = 0.0;
    let mut measured = true;
    for c in text.chars() {
        let code = c as u32;
        width += match code {
            32..=64 => PUNCTUATION[(code - 32) as usize] / 0.6 * DIGIT,
            8192..=10175
            | 11904..=40959
            | 63744..=64255
            | 65040..=65055
            | 65072..=65103
            | 65280..=65376
            | 65471..=65473
            | 65480..=65481
            | 65488..=65489
            | 65496..=65497
            | 65501..=65512
            | 65519 => 1.0,
            65377..=65470
            | 65474..=65479
            | 65482..=65487
            | 65490..=65495
            | 65498..=65500
            | 65513..=65518 => 0.5,
            _ => {
                measured = false;
                if c.is_ascii() {
                    HALF
                } else {
                    1.0
                }
            }
        };
    }
    if measured {
        1.02 * (width * em).ceil()
    } else {
        width * em
    }
}

/// `getBaseDmDuration`: seconds for a message `width` wide to cross a
/// container `container` wide, growing with the container and barely with
/// the message.
fn base_duration(width: f64, container: f64) -> f64 {
    const DEFAULT: f64 = 4.5;
    // (boundary, max, min): containers up to the next boundary use these.
    const STEPS: [(f64, f64, f64); 8] = [
        (558.0, 13.0, 7.0),
        (888.0, 14.0, 8.0),
        (1082.0, 14.0, 9.0),
        (1246.0, 15.0, 10.0),
        (1452.0, 16.0, 11.0),
        (1744.0, 18.0, 13.0),
        (2314.0, 20.0, 14.0),
        (2560.0, 22.0, 16.0),
    ];
    if container <= 0.0 || width <= 0.0 || container < STEPS[0].0 {
        return DEFAULT;
    }
    let (max, min) = match STEPS.windows(2).find(|pair| container <= pair[1].0) {
        Some(pair) => (pair[0].1, pair[0].2),
        None => {
            let (boundary, max, min) = STEPS[STEPS.len() - 1];
            let extra = ((container - boundary) / 500.0).floor().max(0.0);
            (max + extra, min + extra)
        }
    };
    let midpoint = ((max - min) / 0.1 - 1.0).ln() / 0.2 + 0.2;
    min + (max - min) / (1.0 + (-0.2 * (width / container - midpoint)).exp())
}

/// How a message crosses the screen, in container pixels and seconds.
struct Flight {
    width: f64,
    speed: f64,
    /// How far it moves.
    travel: f64,
    duration: f64,
    /// When its left edge reaches the left of the screen.
    middle: f64,
}

impl Flight {
    fn new(now: f64, width: f64) -> Self {
        let speed = (CONTAINER_WIDTH + width) / base_duration(width, CONTAINER_WIDTH);
        let travel = (ENTRY_X + width + SAFE_LEFT).ceil();
        Self {
            width,
            speed,
            travel,
            duration: (travel / speed * 100.0).ceil() / 100.0,
            middle: now + ENTRY_X / speed,
        }
    }
}

/// A message on screen.
struct Placed {
    top: f64,
    bottom: f64,
    width: f64,
    speed: f64,
    middle: f64,
    end: f64,
}

impl Placed {
    /// Its left edge at `now`.
    fn x(&self, now: f64) -> f64 {
        (self.middle - now) * self.speed
    }
}

/// The messages on screen, by bottom, then by when they leave.
#[derive(Default)]
struct Space {
    placed: Vec<Placed>,
}

impl Space {
    /// Bilibili's space manager for scrolling messages: the first top, from
    /// the top of the screen down, where a message entering at `now` overlaps
    /// no message on screen, or follows it closely enough never to catch up
    /// with it. Nothing when it would go below the screen, as the room drops
    /// those.
    fn place(&mut self, now: f64, flight: &Flight, height: f64) -> Option<f64> {
        self.placed.retain(|other| other.end > now);
        let limit = CONTAINER_HEIGHT + 1.0;
        let (mut top, mut bottom) = (0.0, height);
        let mut found = None;
        for (index, other) in self.placed.iter().enumerate() {
            if bottom > limit {
                break;
            }
            if other.bottom <= top {
                continue;
            }
            if other.top > bottom {
                if !self.overlaps(index + 1, top, bottom) {
                    found = Some(top);
                    break;
                }
                continue;
            }
            if ENTRY_X > other.x(now) + other.width && flight.middle > other.end + FOLLOW_GAP {
                if bottom <= other.bottom && !self.overlaps(index + 1, top, bottom) {
                    found = Some(top);
                    break;
                }
                continue;
            }
            top = other.bottom + 1.0;
            bottom = top + height;
            if bottom > limit {
                return None;
            }
        }
        let top = found.or((bottom <= limit).then_some(top))?;
        let placed = Placed {
            top,
            bottom: top + height,
            width: flight.width,
            speed: flight.speed,
            middle: flight.middle,
            end: now + flight.duration,
        };
        let index = self
            .placed
            .partition_point(|other| (other.bottom, other.end) <= (placed.bottom, placed.end));
        self.placed.insert(index, placed);
        Some(top)
    }

    /// Whether a message from `from` on overlaps `top..bottom`. Bilibili
    /// looks at the first only, which misses taller ones (images) further
    /// down the list.
    fn overlaps(&self, from: usize, top: f64, bottom: f64) -> bool {
        self.placed[from..]
            .iter()
            .any(|other| other.bottom >= top && other.top <= bottom)
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

    /// The left-edge path and middle height of each text event.
    fn moves(ass: &str) -> Vec<(String, f64, f64, f64)> {
        ass.lines()
            .filter_map(|line| line.strip_prefix("Dialogue: "))
            .map(|line| {
                let (_, rest) = line.split_once("\\move(").unwrap();
                let (args, text) = rest.split_once(")}").unwrap();
                let args: Vec<f64> = args.split(',').map(|n| n.parse().unwrap()).collect();
                (text.to_string(), args[0], args[2], args[1])
            })
            .collect()
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
        // Text sits below the 54-unit-high image, 1 px apart, in the middle
        // of its row.
        let moves = moves(&plan.ass);
        assert_eq!(moves.len(), 1);
        let (text, _, _, y) = &moves[0];
        assert_eq!(text, "hello");
        assert_eq!(*y, coordinate((82.0 + 47.0 / 2.0) * PX));
    }

    #[test]
    fn failed_images_fall_back_to_normal_text_layout() {
        let entries = vec![entry(1000, "[emoji]"), entry(1000, "hello")];
        let plan = danmu_render_plan(&entries, &Danmu2AssOptions::default(), |_| None);
        assert!(plan.images.is_empty());
        let moves = moves(&plan.ass);
        assert_eq!(moves[0].0, "[emoji]");
        assert_eq!(moves[0].3, coordinate(47.0 / 2.0 * PX));
        // Rows are 1.125 ems high and 1 px apart.
        assert_eq!(moves[1].0, "hello");
        assert_eq!(moves[1].3, coordinate((48.0 + 47.0 / 2.0) * PX));
    }

    #[test]
    fn messages_enter_and_leave_beyond_the_screen() {
        let plan = danmu_render_plan(&[entry(0, "你好")], &Danmu2AssOptions::default(), |_| {
            None
        });
        let (_, from, to, _) = moves(&plan.ass)[0].clone();
        let width = text_width("你好", 36.0 * FONT_EM / PX);
        assert_eq!(from, coordinate((1920.0 + 51.0) * PX));
        assert_eq!(
            to,
            coordinate((1971.0 - (1971.0 + width + 50.0).ceil()) * PX)
        );
    }

    #[test]
    fn rows_are_reused_once_the_message_ahead_cannot_be_caught() {
        let close = [entry(0, "第一条弹幕"), entry(500, "第二条弹幕")];
        let plan = danmu_render_plan(&close, &Danmu2AssOptions::default(), |_| None);
        let rows = moves(&plan.ass);
        assert!(rows[1].3 > rows[0].3);

        let apart = [entry(0, "第一条弹幕"), entry(3000, "第二条弹幕")];
        let plan = danmu_render_plan(&apart, &Danmu2AssOptions::default(), |_| None);
        let rows = moves(&plan.ass);
        assert_eq!(rows[1].3, rows[0].3);
    }

    #[test]
    fn a_full_screen_drops_messages_instead_of_delaying_them() {
        // A row frees up for the same message about 3.9 s after it entered.
        let entries: Vec<_> = (0..30)
            .map(|i| entry(i * 100, "一条很长很长很长很长很长很长的弹幕"))
            .collect();
        let plan = danmu_render_plan(&entries, &Danmu2AssOptions::default(), |_| None);
        let moves = moves(&plan.ass);
        // 22 rows of 47 + 1 px fit in 1081 px.
        assert_eq!(moves.len(), 22);
        assert!(moves.iter().all(|(_, _, _, y)| *y < PLAY_RES_Y));
        let starts: Vec<_> = plan
            .ass
            .lines()
            .filter(|line| line.starts_with("Dialogue: "))
            .map(|line| line.split(',').nth(1).unwrap().to_string())
            .collect();
        assert_eq!(starts[21], format_time(2.1));
    }

    #[test]
    fn messages_of_mixed_sizes_never_cross() {
        // A fixed pseudo-random stream of rows and taller images.
        let mut seed: u64 = 7;
        let mut random = || {
            seed = seed * 48271 % 2_147_483_647;
            seed as f64 / 2_147_483_647.0
        };
        for run in 0..20 {
            let mut space = Space::default();
            let mut placed = Vec::new();
            let mut now = 0.0;
            for _ in 0..150 {
                now += random() * 0.3;
                let (width, height) = if random() < 0.7 {
                    (40.0 + random() * 700.0, 47.0)
                } else {
                    (48.0 + random() * 100.0, 48.0 + random() * 60.0)
                };
                let flight = Flight::new(now, width);
                if let Some(top) = space.place(now, &flight, height) {
                    placed.push((now, width, height, top, flight.travel, flight.duration));
                }
            }
            // As ASS and FFmpeg move them, within 1 px: the duration is
            // rounded up to 0.01 s, a hair slower than the layout reckons.
            let x = |(start, _, _, _, travel, duration): (f64, f64, f64, f64, f64, f64), t: f64| {
                ENTRY_X - travel * (t - start) / duration
            };
            for (i, &a) in placed.iter().enumerate() {
                for &b in &placed[i + 1..] {
                    if a.3 >= b.3 + b.2 || b.3 >= a.3 + a.2 {
                        continue;
                    }
                    let mut t = a.0.max(b.0);
                    while t <= (a.0 + a.5).min(b.0 + b.5) {
                        assert!(
                            !(x(a, t) < x(b, t) + b.1 - 1.0 && x(b, t) < x(a, t) + a.1 - 1.0),
                            "run {run}: {a:?} and {b:?} cross at {t}"
                        );
                        t += 0.05;
                    }
                }
            }
        }
    }

    #[test]
    fn bursts_are_spread_out_and_their_oldest_dropped() {
        let drawn = pace((0..40).map(|i| (0.0, i)).collect());
        // 10 timers and 21 waiting; the 9 oldest waiting were dropped.
        assert_eq!(drawn.len(), 31);
        let items: Vec<_> = drawn.iter().map(|(_, item)| *item).collect();
        assert_eq!(&items[..10], &(0..10).collect::<Vec<_>>()[..]);
        assert_eq!(items[10], 19);
        let times: Vec<_> = drawn.iter().map(|(time, _)| coordinate(*time)).collect();
        assert_eq!(
            &times[..11],
            &[0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 0.9]
        );
        assert!(times.windows(2).all(|pair| pair[0] <= pair[1]));

        let quiet = pace(vec![(0.0, 'a'), (0.5, 'b'), (2.0, 'c')]);
        assert_eq!(quiet, [(0.0, 'a'), (0.5, 'b'), (2.0, 'c')]);
    }

    #[test]
    fn crossing_time_follows_the_container_width() {
        assert_eq!(base_duration(100.0, 400.0), 4.5);
        let full_hd = base_duration(300.0, 1920.0);
        assert!((13.0..13.2).contains(&full_hd), "{full_hd}");
        let window = base_duration(300.0, 1280.0);
        assert!((10.0..10.2).contains(&window), "{window}");
        assert!(base_duration(1500.0, 1920.0) > full_hd);
    }

    #[test]
    fn widths_follow_bilibilis_table() {
        assert_eq!(text_width("你好", 40.0), 1.02 * 80.0);
        assert_eq!(text_width("99", 40.0), 1.02 * 44.0);
        // Not in the table: estimated, as Bilibili renders it.
        assert_eq!(text_width("ab", 40.0), 44.0);
        assert!(text_width("你好", 40.0) > text_width("hi", 40.0));
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
