//! Strict parser for the ASS subset emitted by the application's two generators.
//! Unsupported rendering instructions are errors, never silently discarded.

use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub(crate) struct Style {
    pub font: String,
    pub size: f32,
    pub primary: u32,
    pub outline_color: u32,
    pub back: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub scale_x: f32,
    pub scale_y: f32,
    pub spacing: f32,
    pub angle: f32,
    pub border: u8,
    pub outline: f32,
    pub shadow: f32,
    pub alignment: u8,
    pub margins: [f32; 3],
}

#[derive(Clone, Debug)]
pub(crate) struct Run {
    pub text: String,
    pub font: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub color: u32,
}

type Movement = Option<[(f32, f32); 2]>;

#[derive(Clone, Debug)]
pub(crate) struct Event {
    pub start: f64,
    pub end: f64,
    pub layer: i32,
    pub order: usize,
    pub style: Style,
    pub runs: Vec<Run>,
    pub movement: Movement,
    pub res: (f32, f32),
    pub kind: crate::SourceKind,
}

const STYLE_FIELDS: &[&str] = &[
    "name",
    "fontname",
    "fontsize",
    "primarycolour",
    "secondarycolour",
    "outlinecolour",
    "backcolour",
    "bold",
    "italic",
    "underline",
    "strikeout",
    "scalex",
    "scaley",
    "spacing",
    "angle",
    "borderstyle",
    "outline",
    "shadow",
    "alignment",
    "marginl",
    "marginr",
    "marginv",
    "encoding",
];
const EVENT_FIELDS: &[&str] = &[
    "layer", "start", "end", "style", "name", "marginl", "marginr", "marginv", "effect", "text",
];

// Match the subtitle style editor's numeric domain. Pixel-dependent allocation
// limits belong to the renderer, not this script-coordinate parser.
const NUMBER_LIMIT: f64 = 1_000_000.0;

fn number(raw: &str) -> Result<f32, String> {
    let raw = raw.trim();
    let unsigned = raw.strip_prefix(['+', '-']).unwrap_or(raw);
    let mut dots = 0;
    let mut digits = 0;
    for byte in unsigned.bytes() {
        match byte {
            b'0'..=b'9' => digits += 1,
            b'.' => dots += 1,
            _ => return Err(format!("Invalid decimal number: {raw:?}")),
        }
    }
    if digits == 0 || dots > 1 {
        return Err(format!("Invalid decimal number: {raw:?}"));
    }
    let value: f64 = raw
        .parse()
        .map_err(|_| format!("Invalid number: {raw:?}"))?;
    if !value.is_finite() || value.abs() > NUMBER_LIMIT {
        return Err(format!("Number outside +/-{NUMBER_LIMIT}: {raw:?}"));
    }
    let result = value as f32;
    if value != 0.0 && result == 0.0 {
        return Err(format!("Number too small: {raw:?}"));
    }
    Ok(result)
}

fn nonnegative(raw: &str) -> Result<f32, String> {
    let value = number(raw)?;
    if value < 0.0 {
        Err(format!("Negative value: {raw:?}"))
    } else {
        Ok(value)
    }
}

fn positive(raw: &str) -> Result<f32, String> {
    let value = number(raw)?;
    if value <= 0.0 {
        Err(format!("Expected positive value: {raw:?}"))
    } else {
        Ok(value)
    }
}

fn integer(raw: &str) -> Result<i32, String> {
    // Check before f32 conversion: rounding must not turn a fraction into an integer.
    number(raw)?;
    let value: f64 = raw.trim().parse().map_err(|_| "Invalid integer")?;
    if value.fract() != 0.0 {
        Err(format!("Expected integer: {raw:?}"))
    } else {
        Ok(value as i32)
    }
}

fn margin(raw: &str) -> Result<f32, String> {
    let value = integer(raw)?;
    if value < 0 {
        Err("Margins cannot be negative".into())
    } else {
        Ok(value as f32)
    }
}

fn flag(raw: &str) -> Result<bool, String> {
    match integer(raw)? {
        0 => Ok(false),
        -1 | 1 => Ok(true),
        _ => Err(format!("Expected 0, 1 or -1 flag: {raw:?}")),
    }
}

/// ASS stores inverse-alpha BGR; the renderer consumes ordinary ARGB.
fn color(raw: &str) -> Result<u32, String> {
    let raw = raw.trim();
    let hex = raw
        .strip_prefix("&H")
        .or_else(|| raw.strip_prefix("&h"))
        .ok_or_else(|| format!("Invalid ASS color: {raw:?}"))?;
    let hex = hex.strip_suffix('&').unwrap_or(hex);
    if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("Expected &HBBGGRR or &HAABBGGRR: {raw:?}"));
    }
    let value = u32::from_str_radix(hex, 16).map_err(|_| "Invalid ASS color")?;
    Ok(((255 - (value >> 24)) << 24)
        | ((value & 255) << 16)
        | (value & 0xff00)
        | ((value >> 16) & 255))
}

fn name(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.is_empty()
        || value.chars().count() > 200
        || value.starts_with('*')
        || value
            .chars()
            .any(|c| c.is_control() || ",\\{}[]".contains(c))
    {
        return Err(format!("Invalid style/font name: {raw:?}"));
    }
    Ok(value.into())
}

fn style(fields: &[&str]) -> Result<(String, Style), String> {
    let name = name(fields[0])?;
    let font = self::name(fields[1])?;
    // Secondary colour is only used by karaoke, which is unsupported. Encoding
    // is a legacy codepage hint; input has already been decoded as UTF-8.
    color(fields[4])?;
    margin(fields[22])?;
    let border = integer(fields[15])?;
    if !matches!(border, 1 | 3) {
        return Err("Unsupported BorderStyle (expected 1 or 3)".into());
    }
    let alignment = integer(fields[18])?;
    if !(1..=9).contains(&alignment) {
        return Err("Alignment must be 1..9".into());
    }
    Ok((
        name,
        Style {
            font,
            size: positive(fields[2])?,
            primary: color(fields[3])?,
            outline_color: color(fields[5])?,
            back: color(fields[6])?,
            bold: flag(fields[7])?,
            italic: flag(fields[8])?,
            underline: flag(fields[9])?,
            strike: flag(fields[10])?,
            scale_x: positive(fields[11])? / 100.0,
            scale_y: positive(fields[12])? / 100.0,
            spacing: number(fields[13])?,
            angle: number(fields[14])?,
            border: border as u8,
            outline: nonnegative(fields[16])?,
            shadow: nonnegative(fields[17])?,
            alignment: alignment as u8,
            margins: [
                margin(fields[19])?,
                margin(fields[20])?,
                margin(fields[21])?,
            ],
        },
    ))
}

fn timestamp(raw: &str) -> Result<f64, String> {
    let parts: Vec<_> = raw.trim().split([':', '.']).collect();
    if parts.len() != 4
        || parts[0].is_empty()
        || parts[1..].iter().any(|p| p.len() != 2)
        || parts.iter().any(|p| !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(format!("Expected H:MM:SS.cc time: {raw:?}"));
    }
    let values: Vec<u64> = parts
        .iter()
        .map(|p| p.parse::<u64>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("Time out of range: {raw:?}"))?;
    // danmu2ass rounds seconds independently, so a minute boundary can be
    // emitted as 0:00:60.00. Normalize this exact rollover, not arbitrary SS.
    if values[1] >= 60 || values[2] > 60 || (values[2] == 60 && values[3] != 0) {
        return Err(format!("Invalid minute/second in time: {raw:?}"));
    }
    let centiseconds = values[0]
        .checked_mul(360_000)
        .and_then(|v| v.checked_add(values[1] * 6000 + values[2] * 100 + values[3]))
        .filter(|v| *v <= 9_007_199_254_740_991)
        .ok_or_else(|| format!("Time out of range: {raw:?}"))?;
    Ok(centiseconds as f64 / 100.0)
}

/// Maps canonical field indices to declared indices, validating the whole schema.
fn format(raw: &str, expected: &[&str]) -> Result<Vec<usize>, String> {
    let fields: Vec<_> = raw
        .split(',')
        .map(|v| v.trim().to_ascii_lowercase())
        .collect();
    if fields.len() != expected.len() {
        return Err("Wrong number of Format fields".into());
    }
    let mut result = Vec::new();
    for field in expected {
        if fields.iter().filter(|v| v.as_str() == *field).count() != 1 {
            return Err(format!("Format must contain exactly one {field}"));
        }
        result.push(fields.iter().position(|v| v == field).unwrap());
    }
    Ok(result)
}

fn row<'a>(raw: &'a str, format: &[usize], text: Option<usize>) -> Result<Vec<&'a str>, String> {
    // Split around Text from both ends. Even a reordered Text column can
    // contain commas; all remaining fields have an unambiguous fixed count.
    let fields = if let Some(text) = text {
        let index = format[text];
        let mut prefix: Vec<_> = raw.splitn(index + 1, ',').collect();
        let rest = prefix.pop().ok_or("Missing event fields")?;
        let mut suffix: Vec<_> = rest.rsplitn(format.len() - index, ',').collect();
        suffix.reverse();
        prefix.extend(suffix);
        prefix
    } else {
        raw.split(',').collect()
    };
    if fields.len() != format.len() {
        return Err("Wrong number of row fields".into());
    }
    Ok(format.iter().map(|i| fields[*i]).collect())
}

fn flush(runs: &mut Vec<Run>, run: &mut Run) {
    if !run.text.is_empty() {
        runs.push(Run {
            text: std::mem::take(&mut run.text),
            ..run.clone()
        });
    }
}

fn override_tags(
    block: &str,
    run: &mut Run,
    style: &Style,
    movement: &mut Movement,
) -> Result<(), String> {
    let tags = block
        .strip_prefix('\\')
        .ok_or("Override block must contain tags")?;
    for tag in tags.split('\\') {
        if let Some(args) = tag.strip_prefix("move(").and_then(|v| v.strip_suffix(')')) {
            if movement.is_some() {
                return Err("Multiple move tags are unsupported".into());
            }
            let args: Vec<_> = args.split(',').map(number).collect::<Result<_, _>>()?;
            if args.len() != 4 {
                return Err("move requires exactly four coordinates".into());
            }
            *movement = Some([(args[0], args[1]), (args[2], args[3])]);
        } else if let Some(value) = tag.strip_prefix("fn") {
            run.font = if value.is_empty() {
                style.font.clone()
            } else {
                name(value)?
            };
        } else if let Some(value) = tag.strip_prefix("fs") {
            run.size = if value.is_empty() {
                style.size
            } else {
                positive(value)?
            };
        } else if let Some(value) = tag.strip_prefix('c') {
            run.color = if value.is_empty() {
                style.primary
            } else {
                // \c only changes RGB, never the alpha inherited from the style.
                let rgb = color(value)?;
                if value.trim_end_matches('&').len() != 8 {
                    return Err("Inline c requires a six-digit RGB color".into());
                }
                (style.primary & 0xff00_0000) | (rgb & 0x00ff_ffff)
            };
        } else {
            let (tag_name, value) =
                tag.split_at(tag.chars().next().map(char::len_utf8).unwrap_or(0));
            let target = match tag_name {
                "b" => &mut run.bold,
                "i" => &mut run.italic,
                "u" => &mut run.underline,
                "s" => &mut run.strike,
                _ => return Err(format!("Unsupported ASS override: \\{tag}")),
            };
            *target = match value {
                "0" => false,
                "1" | "-1" => true,
                _ => return Err(format!("Unsupported ASS toggle: \\{tag}")),
            };
        }
    }
    Ok(())
}

fn text(raw: &str, style: &Style) -> Result<(Vec<Run>, Movement), String> {
    let mut run = Run {
        text: String::new(),
        font: style.font.clone(),
        size: style.size,
        bold: style.bold,
        italic: style.italic,
        underline: style.underline,
        strike: style.strike,
        color: style.primary,
    };
    let mut runs = Vec::new();
    let mut movement = None;
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                flush(&mut runs, &mut run);
                let mut block = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some('{') | None => return Err("Malformed ASS override block".into()),
                        Some(c) => block.push(c),
                    }
                }
                override_tags(&block, &mut run, style, &mut movement)?;
            }
            '}' => return Err("Unescaped closing brace".into()),
            '\\' => match chars.next() {
                Some('N') => run.text.push('\n'),
                Some('\u{2060}') | Some('\\') => run.text.push('\\'),
                Some(c @ ('{' | '}')) => run.text.push(c),
                Some(c) => return Err(format!("Unsupported ASS escape: \\{c}")),
                None => return Err("Trailing unescaped backslash".into()),
            },
            c if c.is_control() && c != '\t' => {
                return Err("Control character in event text".into())
            }
            c => run.text.push(c),
        }
    }
    flush(&mut runs, &mut run);
    Ok((runs, movement))
}

pub(crate) fn parse(source: &str, kind: crate::SourceKind) -> Result<Vec<Event>, String> {
    let mut section = 0;
    let mut seen_sections = HashSet::new();
    let mut info = HashSet::new();
    let mut res_x = None;
    let mut res_y = None;
    let mut styles = HashMap::new();
    let mut style_format = None;
    let mut event_format = None;
    let mut events = Vec::new();
    for (line_number, raw) in source.trim_start_matches('\u{feff}').lines().enumerate() {
        // Preserve trailing event text spaces, while allowing indented headers.
        let line = raw.trim_start();
        if line.trim().is_empty() || line.starts_with(';') {
            continue;
        }
        let result: Result<(), String> = (|| {
            if line.starts_with('[') {
                let next = match line.trim().to_ascii_lowercase().as_str() {
                    "[script info]" => 1,
                    "[v4+ styles]" => 2,
                    "[events]" => 3,
                    _ => return Err(format!("Unsupported ASS section: {line}")),
                };
                if next != section + 1 || !seen_sections.insert(next) {
                    return Err(
                        "Expected Script Info, V4+ Styles, Events in order, once each".into(),
                    );
                }
                section = next;
                return Ok(());
            }
            let (key, value) = line.split_once(':').ok_or("Expected key: value")?;
            let key = key.trim().to_ascii_lowercase();
            match section {
                1 => {
                    if !info.insert(key.clone()) {
                        return Err(format!("Duplicate metadata: {key}"));
                    }
                    let value = value.trim();
                    match key.as_str() {
                        "title" => {}
                        "scripttype" if value.eq_ignore_ascii_case("v4.00+") => {}
                        "playresx" => res_x = Some(positive(value)?),
                        "playresy" => res_y = Some(positive(value)?),
                        "wrapstyle" if integer(value)? == 0 => {}
                        "scaledborderandshadow" if value.eq_ignore_ascii_case("yes") => {}
                        "ycbcr matrix" if value.eq_ignore_ascii_case("none") => {}
                        "collisions" if value.eq_ignore_ascii_case("normal") => {}
                        // libass ignores this legacy playback-speed hint. The
                        // danmu generator emits 10; standard ASS uses 100.
                        "timer" if matches!(number(value)?, 10.0 | 100.0) => {}
                        _ => {
                            return Err(format!("Unsupported Script Info setting: {key}: {value}"))
                        }
                    }
                }
                2 => match key.as_str() {
                    "format" if style_format.is_none() && styles.is_empty() => {
                        style_format = Some(format(value, STYLE_FIELDS)?);
                    }
                    "style" => {
                        let fields = row(
                            value,
                            style_format.as_deref().ok_or("Style before Format")?,
                            None,
                        )?;
                        let (name, style) = style(&fields)?;
                        if styles.insert(name, style).is_some() {
                            return Err("Duplicate style name".into());
                        }
                    }
                    _ => return Err(format!("Unsupported styles record: {key}")),
                },
                3 => match key.as_str() {
                    "format" if event_format.is_none() && events.is_empty() => {
                        event_format = Some(format(value, EVENT_FIELDS)?);
                    }
                    "dialogue" => {
                        let fields = row(
                            value,
                            event_format.as_deref().ok_or("Dialogue before Format")?,
                            Some(9),
                        )?;
                        if !fields[8].trim().is_empty() {
                            return Err("ASS event effects are unsupported".into());
                        }
                        let mut style = styles
                            .get(fields[3].trim())
                            .ok_or("Unknown event style")?
                            .clone();
                        for (index, field) in fields[5..8].iter().enumerate() {
                            let value = margin(field)?;
                            if value != 0.0 {
                                style.margins[index] = value;
                            }
                        }
                        let start = timestamp(fields[1])?;
                        let end = timestamp(fields[2])?;
                        if end <= start {
                            return Err("Event end must be after start".into());
                        }
                        let (runs, movement) = text(fields[9], &style)?;
                        events.push(Event {
                            start,
                            end,
                            layer: integer(fields[0])?,
                            order: events.len(),
                            style,
                            runs,
                            movement,
                            res: (
                                res_x.ok_or("Missing PlayResX")?,
                                res_y.ok_or("Missing PlayResY")?,
                            ),
                            kind,
                        });
                    }
                    _ => return Err(format!("Unsupported events record: {key}")),
                },
                _ => return Err("Record outside an ASS section".into()),
            }
            Ok(())
        })();
        result.map_err(|error| format!("ASS line {}: {error}", line_number + 1))?;
    }
    if section != 3
        || style_format.is_none()
        || event_format.is_none()
        || styles.is_empty()
        || res_x.is_none()
        || res_y.is_none()
        || !info.contains("scripttype")
    {
        return Err("Incomplete generated ASS document".into());
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STYLE: &str = "Default,Arial,64,&H80112233,&H00FFFFFF,&H40224466,&HFF000000,-1,1,0,-1,125,80,1.5,-12,3,4.5,2,8,10,20,30,1";

    fn document(events: &str) -> String {
        format!("[Script Info]\nScriptType: v4.00+\nPlayResX: 1920\nPlayResY: 1080\nWrapStyle: 0\nScaledBorderAndShadow: yes\nYCbCr Matrix: None\n\n[V4+ Styles]\nFormat: {}\nStyle: {STYLE}\n\n[Events]\nFormat: {}\n{events}\n", STYLE_FIELDS.join(", "), EVENT_FIELDS.join(", "))
    }

    fn parse_text(text: &str) -> Result<Vec<Event>, String> {
        parse(
            &document(&format!(
                "Dialogue: 0,0:00:01.25,0:00:03.50,Default,,0,0,0,,{text}"
            )),
            crate::SourceKind::Subtitle,
        )
    }

    #[test]
    fn preserves_style_fields_and_converts_colors() {
        let event = parse_text("hello").unwrap().remove(0);
        let s = event.style;
        assert_eq!(s.font, "Arial");
        assert_eq!(s.size, 64.0);
        assert_eq!(s.primary, 0x7f332211);
        assert_eq!(s.outline_color, 0xbf664422);
        assert_eq!(s.back, 0);
        assert!(s.bold && s.italic && !s.underline && s.strike);
        assert_eq!(
            (s.scale_x, s.scale_y, s.spacing, s.angle),
            (1.25, 0.8, 1.5, -12.0)
        );
        assert_eq!(
            (s.border, s.outline, s.shadow, s.alignment),
            (3, 4.5, 2.0, 8)
        );
        assert_eq!(s.margins, [10.0, 20.0, 30.0]);
        assert_eq!(event.res, (1920.0, 1080.0));
        assert_eq!((event.start, event.end), (1.25, 3.5));
    }

    #[test]
    fn inline_toggles_and_color_reset_preserve_style_alpha() {
        let events = parse_text(r"a{\b0\i0\u1\s0\c&HABCDEF&}b{\c}c").unwrap();
        let runs = &events[0].runs;
        assert_eq!(runs.len(), 3);
        assert!(runs[0].bold && runs[0].italic && runs[0].strike);
        assert!(!runs[1].bold && !runs[1].italic && runs[1].underline && !runs[1].strike);
        assert_eq!(runs[1].color, 0x7fefcdab);
        assert_eq!(runs[2].color, 0x7f332211);
    }

    #[test]
    fn escapes_are_literal_not_reparsed_as_tags() {
        let events = parse_text("  \\{\\\\b1\\} \\\u{2060}N \\\\N\\N中文,  ").unwrap();
        assert_eq!(events[0].runs[0].text, "  {\\b1} \\N \\N\n中文,  ");
        assert!(!events[0].runs[0].underline);
    }

    #[test]
    fn preserves_read_order_layer_margins_and_move() {
        let events = parse(&document(concat!(
            "Dialogue: 2,0:02:00.00,0:02:10.00,Default,,0,99,40,,{\\move(1280,22.5,-100,22.5)}later\n",
            "Dialogue: -1,0:00:00.00,0:00:60.00,Default,,0,0,0,,earlier"
        )), crate::SourceKind::Subtitle).unwrap();
        assert_eq!((events[0].order, events[1].order), (0, 1));
        assert_eq!((events[0].layer, events[1].layer), (2, -1));
        assert_eq!(events[0].style.margins, [10.0, 99.0, 40.0]);
        assert_eq!(events[0].movement, Some([(1280.0, 22.5), (-100.0, 22.5)]));
        assert_eq!(events[1].end, 60.0);
    }

    #[test]
    fn accepts_danmu_metadata_and_empty_document() {
        let doc = document("").replace(
            "WrapStyle: 0\nScaledBorderAndShadow: yes\nYCbCr Matrix: None",
            "Title: Bilibili Danmaku\nCollisions: Normal\nTimer: 10.0000",
        );
        assert!(parse(&doc, crate::SourceKind::Subtitle).unwrap().is_empty());
    }

    #[test]
    fn parses_generated_danmu_with_bom_and_crlf() {
        let doc = format!(
            "\u{feff}[Script Info]\nTitle: Bilibili Danmaku\nScriptType: v4.00+\nCollisions: Normal\nPlayResX: 1280\nPlayResY: 720\nTimer: 10.0000\n\n[V4+ Styles]\nFormat: {}\nStyle: Default,微软雅黑,36,&H32FFFFFF,&H32FFFFFF,&H32000000,&H32000000,-1,0,0,0,100,100,0,0,1,1,0,4,20,20,2,0\n\n[Events]\nFormat: {}\nDialogue: 0,0:00:00.00,0:00:14.31,Default,,0,0,0,,{{\\move(1314,15.67,-123.33,15.67)}}弹幕\\\\N\\N｛test｝\n",
            STYLE_FIELDS.join(", "), EVENT_FIELDS.join(", ")
        ).replace('\n', "\r\n");
        let events = parse(&doc, crate::SourceKind::Subtitle).unwrap();
        assert_eq!(events[0].res, (1280.0, 720.0));
        assert_eq!(events[0].style.font, "微软雅黑");
        assert_eq!(events[0].style.primary, 0xcdffffff);
        assert_eq!(events[0].runs[0].text, "弹幕\\N\n｛test｝");
        assert!(events[0].runs[0].bold);
        assert!(matches!(events[0].kind, crate::SourceKind::Subtitle));
    }

    #[test]
    fn respects_reordered_formats_including_comma_text() {
        let style_fields: Vec<_> = STYLE.split(',').collect();
        let event = [
            "7",
            "0:00:01.00",
            "0:00:02.00",
            "Default",
            "",
            "0",
            "0",
            "0",
            "",
            "a,b",
        ];
        let doc = document("")
            .replace(
                &STYLE_FIELDS.join(", "),
                &STYLE_FIELDS
                    .iter()
                    .rev()
                    .copied()
                    .collect::<Vec<_>>()
                    .join(", "),
            )
            .replace(
                STYLE,
                &style_fields.into_iter().rev().collect::<Vec<_>>().join(","),
            )
            .replace(
                &EVENT_FIELDS.join(", "),
                &EVENT_FIELDS
                    .iter()
                    .rev()
                    .copied()
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        let doc = format!(
            "{doc}Dialogue:{}",
            event.into_iter().rev().collect::<Vec<_>>().join(",")
        );
        let events = parse(&doc, crate::SourceKind::Subtitle).unwrap();
        assert_eq!(events[0].layer, 7);
        assert_eq!(events[0].runs[0].text, "a,b");
    }

    #[test]
    fn inline_font_and_size_are_bounded_and_reset_to_style() {
        let events = parse_text(r"a{\fnNoto Sans CJK SC\fs32.5}中😀{\fn\fs}z").unwrap();
        let runs = &events[0].runs;
        assert_eq!((runs[0].font.as_str(), runs[0].size), ("Arial", 64.0));
        assert_eq!(
            (runs[1].font.as_str(), runs[1].size),
            ("Noto Sans CJK SC", 32.5)
        );
        assert_eq!((runs[2].font.as_str(), runs[2].size), ("Arial", 64.0));
        for text in [
            r"{\fs0}a",
            r"{\fs-1}a",
            r"{\fsNaN}a",
            r"{\fs1000001}a",
            r"{\fnA,B}a",
        ] {
            assert!(parse_text(text).is_err(), "accepted {text}");
        }
    }

    #[test]
    fn rejects_unsupported_and_malformed_text() {
        for text in [
            r"{\pos(1,2)}a",
            r"{\an7}a",
            r"{\fad(1,2)}a",
            r"{\p1}m 0 0",
            r"{\t(1,2,\b1)}a",
            r"{\b700}a",
            r"{\c&H00112233&}a",
            r"{\move(1,2,3,4,5,6)}a",
            r"{\move(1,2,3,4)\move(1,2,3,4)}a",
            r"{\move(NaN,2,3,4)}a",
            r"{\b1",
            "a}",
            "{text}",
            "{}",
            r"\h",
            r"\n",
            "a\\",
        ] {
            assert!(parse_text(text).is_err(), "accepted {text:?}");
        }
    }

    #[test]
    fn rejects_invalid_numbers_times_and_document_features() {
        for value in ["NaN", "inf", "1e2", "1000001", "--1", ".", "1.2.3"] {
            assert!(number(value).is_err(), "accepted {value}");
        }
        assert!(integer("999999.999").is_err());
        for value in [
            "-1:00:00.00",
            "0:60:00.00",
            "0:00:60.01",
            "0:00:61.00",
            "0:00:01.000",
            "0:0:01.00",
            "999999999999999999999:00:00.00",
        ] {
            assert!(timestamp(value).is_err(), "accepted {value}");
        }
        for (old, new) in [
            (",Arial,64,", ",Arial,0,"),
            (",125,80,", ",0,80,"),
            (",3,4.5,2,8,", ",2,4.5,2,8,"),
            (",3,4.5,2,8,", ",3,-1,2,8,"),
            (",3,4.5,2,8,", ",3,4.5,2,10,"),
            (",-1,1,0,-1,", ",2,1,0,-1,"),
            ("0:00:03.50", "0:00:01.25"),
            ("Default,,0,0,0,,", "Missing,,0,0,0,,"),
            ("Default,,0,0,0,,", "Default,,0,0,0,Scroll up;1;2;3,"),
            ("WrapStyle: 0", "WrapStyle: 2"),
            ("YCbCr Matrix: None", "YCbCr Matrix: TV.601"),
            ("[Events]", "[Fonts]"),
            ("layer, start", "layer, layer"),
        ] {
            let doc = document("Dialogue: 0,0:00:01.25,0:00:03.50,Default,,0,0,0,,x");
            assert!(doc.contains(old));
            assert!(
                parse(&doc.replace(old, new), crate::SourceKind::Subtitle).is_err(),
                "accepted {new}"
            );
        }
        assert!(parse("", crate::SourceKind::Subtitle).is_err());
    }
}
