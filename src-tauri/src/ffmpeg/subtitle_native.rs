//! Explicit SRT conversion for the native renderer: never ask FFmpeg to emit
//! ASS overrides, since its decoder can emit tags outside our supported subset.
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use regex::Regex;

const SCALE_X: f64 = 1920.0 / 384.0;
const SCALE_Y: f64 = 1080.0 / 288.0;

const FIELDS: [&str; 23] = [
    "Name",
    "Fontname",
    "Fontsize",
    "PrimaryColour",
    "SecondaryColour",
    "OutlineColour",
    "BackColour",
    "Bold",
    "Italic",
    "Underline",
    "StrikeOut",
    "ScaleX",
    "ScaleY",
    "Spacing",
    "Angle",
    "BorderStyle",
    "Outline",
    "Shadow",
    "Alignment",
    "MarginL",
    "MarginR",
    "MarginV",
    "Encoding",
];

fn safe_name(value: &str) -> Result<String, String> {
    if value.is_empty()
        || value.chars().count() > 200
        || value.starts_with('*')
        || value
            .chars()
            .any(|c| c.is_control() || ",\\{}[]".contains(c))
    {
        return Err(
            "Font/style name is empty, too long, or contains ASS control characters".into(),
        );
    }
    Ok(value.into())
}

fn number(value: &str) -> Result<f64, String> {
    static NUMBER: OnceLock<Regex> = OnceLock::new();
    if !NUMBER
        .get_or_init(|| Regex::new(r"^[+-]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)$").unwrap())
        .is_match(value)
    {
        return Err(format!("Invalid numeric value: {value}"));
    }
    let n = value
        .parse::<f64>()
        .map_err(|_| format!("Invalid number: {value}"))?;
    if !n.is_finite() || n.abs() > 1_000_000.0 {
        return Err(format!("Number out of range: {value}"));
    }
    Ok(n)
}

fn positive_number(value: &str) -> Result<String, String> {
    let n = number(value)?;
    if n <= 0.0 {
        return Err("Size/scale must be greater than zero".into());
    }
    Ok(n.to_string())
}

fn ass_color(value: &str) -> Result<String, String> {
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() == 6 && hex.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Ok(format!("&H00{}{}{}", &hex[4..6], &hex[2..4], &hex[0..2]).to_uppercase());
        }
    }
    if value.starts_with("&H") || value.starts_with("&h") {
        let hex = value[2..].strip_suffix('&').unwrap_or(&value[2..]);
        if matches!(hex.len(), 6 | 8) && hex.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Ok(format!("&H{:0>8}", hex.to_uppercase()));
        }
    }
    // ASS also permits signed/unsigned decimal representations of AABBGGRR.
    if let Ok(n) = value.parse::<i64>() {
        if (i32::MIN as i64..=u32::MAX as i64).contains(&n) {
            return Ok(format!("&H{:08X}", n as u32));
        }
    }
    Err(format!("Invalid ASS color: {value}"))
}

fn style_values(force_style: &str) -> Result<Vec<String>, String> {
    // FFmpeg's native SRT defaults, in its original 384 × 288 units.
    let mut values = [
        "Default",
        "Arial",
        "16",
        "&H00FFFFFF",
        "&H00FFFFFF",
        "&H00000000",
        "&H00000000",
        "0",
        "0",
        "0",
        "0",
        "100",
        "100",
        "0",
        "0",
        "1",
        "1",
        "0",
        "2",
        "10",
        "10",
        "10",
        "1",
    ]
    .map(str::to_owned)
    .to_vec();
    let mut seen = HashSet::new();
    if force_style.trim().is_empty() {
        return Ok(values);
    }
    for entry in force_style.split(',') {
        let (key, raw) = entry
            .split_once('=')
            .ok_or_else(|| format!("Expected style field=value: {entry}"))?;
        let key = key.trim();
        let value = raw.trim();
        let index = FIELDS
            .iter()
            .position(|field| field.eq_ignore_ascii_case(key))
            .ok_or_else(|| format!("Unsupported style field: {key}"))?;
        if !seen.insert(index) {
            return Err(format!("Duplicate style field: {key}"));
        }
        values[index] = match index {
            0 | 1 => safe_name(value)?,
            3..=6 => ass_color(value)?,
            2 | 11 | 12 => positive_number(value)?,
            _ => {
                let n = number(value)?;
                if matches!(index, 7..=10 | 15 | 18..=22) && n.fract() != 0.0 {
                    return Err(format!("{key} must be an integer"));
                }
                if matches!(index, 7..=10) && ![-1.0, 0.0, 1.0].contains(&n) {
                    return Err(format!("{key} must be -1, 0, or 1"));
                }
                if index == 15 && n != 1.0 && n != 3.0 {
                    return Err("BorderStyle must be 1 or 3".into());
                }
                if matches!(index, 16 | 17 | 19..=22) && n < 0.0 {
                    return Err(format!("{key} cannot be negative"));
                }
                if index == 18 {
                    match n as i64 {
                        1 => "1",
                        2 => "2",
                        3 => "3",
                        5 => "7",
                        6 => "8",
                        7 => "9",
                        9 => "4",
                        10 => "5",
                        11 => "6",
                        _ => {
                            return Err(
                                "Invalid legacy Alignment (expected 1,2,3,5,6,7,9,10,11)".into()
                            )
                        }
                    }
                    .into()
                } else {
                    n.to_string()
                }
            }
        };
    }
    Ok(values)
}

fn scaled_value(value: &str, factor: f64, integer: bool) -> Result<String, String> {
    let value = number(value)? * factor;
    let value = if integer { value.round() } else { value };
    if !value.is_finite() || value.abs() > 1_000_000.0 || (value != 0.0 && value as f32 == 0.0) {
        return Err("Scaled subtitle value exceeds the native renderer's numeric range".into());
    }
    Ok(value.to_string())
}

fn normalized_style(force_style: &str) -> Result<Vec<String>, String> {
    let mut values = style_values(force_style)?;
    // Match subtitle-ass.ts legacyValues: move the legacy 384x288 settings
    // into the canonical 16:9 script without stretching the font horizontally.
    for index in [2, 13, 16, 17] {
        values[index] = scaled_value(&values[index], SCALE_Y, false)?;
    }
    for index in [19, 20] {
        values[index] = scaled_value(&values[index], SCALE_X, true)?;
    }
    // ASS margins are integer fields; round just as legacyValues does.
    values[21] = scaled_value(&values[21], SCALE_Y, true)?;
    Ok(values)
}

fn html_color(value: &str) -> Result<String, String> {
    let value = value.trim().to_ascii_lowercase();
    let html = match value.as_str() {
        "black" => "#000000",
        "silver" => "#c0c0c0",
        "gray" => "#808080",
        "white" => "#ffffff",
        "maroon" => "#800000",
        "red" => "#ff0000",
        "purple" => "#800080",
        "fuchsia" => "#ff00ff",
        "green" => "#008000",
        "lime" => "#00ff00",
        "olive" => "#808000",
        "yellow" => "#ffff00",
        "navy" => "#000080",
        "blue" => "#0000ff",
        "teal" => "#008080",
        "aqua" => "#00ffff",
        "orange" => "#ffa500",
        other => other,
    };
    let expanded;
    let html = if html.len() == 4
        && html.starts_with('#')
        && html[1..].bytes().all(|c| c.is_ascii_hexdigit())
    {
        expanded = format!(
            "#{}",
            html[1..].chars().flat_map(|c| [c, c]).collect::<String>()
        );
        expanded.as_str()
    } else {
        html
    };
    if !html.starts_with('#') || html.len() != 7 {
        return Err(format!("Unsupported HTML color: {value}"));
    }
    Ok(ass_color(html)?[4..].into())
}

fn literal_text(text: &str, output: &mut String) {
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => output.push_str("\\\u{2060}"),
            '{' => output.push_str("\\{"),
            '}' => output.push_str("\\}"),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                output.push_str("\\N");
            }
            '\n' => output.push_str("\\N"),
            '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{7f}' => {}
            _ => output.push(c),
        }
    }
}

#[derive(Clone, Default)]
struct Font {
    color: Option<String>,
    face: Option<String>,
    size: Option<String>,
}

impl Font {
    fn transition(&self, next: &Self, output: &mut String) {
        if self.color != next.color {
            match &next.color {
                Some(color) => output.push_str(&format!("{{\\c&H{color}&}}")),
                None => output.push_str("{\\c}"),
            }
        }
        if self.face != next.face {
            output.push_str(&format!("{{\\fn{}}}", next.face.as_deref().unwrap_or("")));
        }
        if self.size != next.size {
            output.push_str(&format!("{{\\fs{}}}", next.size.as_deref().unwrap_or("")));
        }
    }
}

fn font_attributes(attributes: &str, current: &Font) -> Result<Font, String> {
    static ATTRIBUTE: OnceLock<Regex> = OnceLock::new();
    let attribute = ATTRIBUTE.get_or_init(|| {
        Regex::new(r#"^\s*([A-Za-z][A-Za-z0-9_-]*)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>/]+))"#)
            .unwrap()
    });
    let mut remaining = attributes;
    let mut next = current.clone();
    let mut seen = HashSet::new();
    while !remaining.trim().is_empty() {
        let captures = attribute
            .captures(remaining)
            .ok_or_else(|| format!("Malformed/unsupported font attributes: {remaining}"))?;
        let name = captures[1].to_ascii_lowercase();
        if !seen.insert(name.clone()) {
            return Err(format!("Duplicate font attribute: {name}"));
        }
        let value = (2..=4)
            .find_map(|i| captures.get(i))
            .unwrap()
            .as_str()
            .trim();
        match name.as_str() {
            "color" => next.color = Some(html_color(value)?),
            "face" => next.face = Some(safe_name(value)?),
            "size" => {
                // SRT sizes here are absolute script units, not HTML's relative
                // +N/-N scale. Reject relative syntax rather than reinterpret it.
                if value.starts_with(['+', '-']) {
                    return Err("Relative font sizes are unsupported".into());
                }
                next.size = Some(scaled_value(&positive_number(value)?, SCALE_Y, false)?);
            }
            _ => return Err(format!("Unsupported font attribute: {name}")),
        }
        remaining = &remaining[captures[0].len()..];
        if !remaining.is_empty() && !remaining.starts_with(char::is_whitespace) {
            return Err("Font attributes must be separated by whitespace".into());
        }
    }
    Ok(next)
}

fn cue_text(text: &str) -> Result<String, String> {
    static TAGS: OnceLock<Regex> = OnceLock::new();
    let tags =
        TAGS.get_or_init(|| Regex::new(r"(?i)<\s*(/?)\s*(b|i|u|s|br|font)\b([^<>]*)>").unwrap());
    let mut output = String::new();
    let mut offset = 0;
    let mut font = Font::default();
    let mut fonts = Vec::new();
    let mut depths = [0usize; 4];
    for tag in tags.captures_iter(text) {
        let whole = tag.get(0).unwrap();
        literal_text(&text[offset..whole.start()], &mut output);
        let name = tag[2].to_ascii_lowercase();
        let closing = !tag[1].is_empty();
        let attributes = tag[3].trim();
        if name == "font" {
            let next = if closing {
                if !attributes.is_empty() {
                    return Err("Closing font tag cannot contain attributes".into());
                }
                fonts.pop().unwrap_or_else(|| font.clone())
            } else {
                let next = font_attributes(attributes, &font)?;
                fonts.push(font.clone());
                next
            };
            font.transition(&next, &mut output);
            font = next;
        } else if name == "br" {
            if !(attributes.is_empty() || attributes == "/") {
                return Err("Unsupported br attributes".into());
            }
            output.push_str("\\N");
        } else {
            if !attributes.is_empty() {
                return Err(format!("Unsupported {name} attributes"));
            }
            let index = match name.as_str() {
                "b" => 0,
                "i" => 1,
                "u" => 2,
                _ => 3,
            };
            if closing {
                depths[index] = depths[index].saturating_sub(1);
            } else {
                depths[index] += 1;
            }
            output.push_str(&format!("{{\\{name}{}}}", usize::from(depths[index] > 0)));
        }
        offset = whole.end();
    }
    literal_text(&text[offset..], &mut output);
    Ok(output)
}

fn milliseconds(time: &srtparse::Time) -> Result<u64, String> {
    if time.minutes >= 60 || time.seconds >= 60 || time.milliseconds >= 1000 {
        return Err("SRT timestamp minutes/seconds/milliseconds out of range".into());
    }
    time.hours
        .checked_mul(60)
        .and_then(|n| n.checked_add(time.minutes))
        .and_then(|n| n.checked_mul(60))
        .and_then(|n| n.checked_add(time.seconds))
        .and_then(|n| n.checked_mul(1000))
        .and_then(|n| n.checked_add(time.milliseconds))
        .ok_or_else(|| "SRT timestamp overflows milliseconds".into())
}

fn centiseconds(ms: u64) -> Result<u64, String> {
    ms.checked_add(5)
        .map(|n| n / 10)
        .ok_or_else(|| "SRT timestamp overflows rounding".into())
}

fn ass_time(cs: u64) -> String {
    format!(
        "{}:{:02}:{:02}.{:02}",
        cs / 360_000,
        cs / 6000 % 60,
        cs / 100 % 60,
        cs % 100
    )
}

/// Parse using the app's SRT parser (including its whitespace/BOM behavior).
/// Invalid/reversed/zero-length source timestamps are errors. Positive cues that
/// collapse after nearest-centisecond rounding are omitted, matching preview.
/// Sort by original millisecond start then original input order, not SRT index.
/// Unrecognized HTML tags remain visible. Recognized tags with unsupported or
/// invalid attributes fail explicitly rather than silently losing formatting.
pub(super) fn srt_to_ass(content: &str, force_style: &str) -> Result<String, String> {
    let values = normalized_style(force_style)?;
    let items = srtparse::from_str(content).map_err(|e| format!("Invalid SRT: {e}"))?;
    if items.is_empty() {
        return Err("SRT contains no subtitle cues".into());
    }
    let mut cues = Vec::with_capacity(items.len());
    for (order, item) in items.into_iter().enumerate() {
        let start = milliseconds(&item.start_time)?;
        let end = milliseconds(&item.end_time)?;
        if end <= start {
            return Err(format!("SRT cue {} must end after it starts", order + 1));
        }
        // Validate even cues that will disappear after time quantization.
        let text = cue_text(&item.text).map_err(|e| format!("SRT cue {}: {e}", order + 1))?;
        cues.push((start, order, centiseconds(start)?, centiseconds(end)?, text));
    }
    cues.sort_by_key(|cue| (cue.0, cue.1));
    let mut output = format!(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: 1920\nPlayResY: 1080\nWrapStyle: 0\nScaledBorderAndShadow: yes\nYCbCr Matrix: None\n\n[V4+ Styles]\nFormat: {}\nStyle: {}\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
        FIELDS.join(", "), values.join(",")
    );
    for (_, _, start, end, text) in cues {
        if end <= start {
            continue;
        }
        output.push_str(&format!(
            "Dialogue: 0,{},{},{},,0,0,0,,{}\n",
            ass_time(start),
            ass_time(end),
            values[0],
            text
        ));
    }
    Ok(output)
}

/// Collect only immediate font files, sorted by path. No recursive traversal.
/// Fail rather than silently truncate beyond 128 fonts or ignore I/O errors.
pub(super) fn font_paths(directory: Option<&Path>) -> Result<Vec<PathBuf>, String> {
    let Some(directory) = directory else {
        return Ok(Vec::new());
    };
    let entries = std::fs::read_dir(directory)
        .map_err(|e| format!("Cannot read fonts directory {}: {e}", directory.display()))?;
    let mut fonts = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("Cannot read font directory entry: {e}"))?;
        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "ttf" | "otf" | "ttc" | "otc") {
            continue;
        }
        let metadata = std::fs::metadata(&path)
            .map_err(|e| format!("Cannot inspect font {}: {e}", path.display()))?;
        if !metadata.is_file() {
            continue;
        }
        std::fs::File::open(&path)
            .map_err(|e| format!("Cannot read font {}: {e}", path.display()))?;
        fonts.push(path);
        if fonts.len() > 128 {
            return Err("Fonts directory exceeds 128 font files".into());
        }
    }
    fonts.sort();
    Ok(fonts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn srt(text: &str) -> String {
        format!("1\n00:00:00,000 --> 00:00:01,000\n{text}\n")
    }

    #[test]
    fn native_defaults_and_quotes_are_literal() {
        let output = srt_to_ass(&srt("It's \"normal\" text"), "").unwrap();
        assert!(output.contains("PlayResX: 1920\nPlayResY: 1080"));
        assert!(output.contains("Style: Default,Arial,60,&H00FFFFFF,&H00FFFFFF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,3.75,0,2,50,50,38,1"));
        assert!(output
            .ends_with("Dialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,It's \"normal\" text\n"));
    }

    #[test]
    fn canonical_resolution_scales_legacy_lengths_and_inline_sizes() {
        let style = "FontSize=18,Outline=2,Shadow=1,Spacing=1.5,MarginL=20,MarginR=22,MarginV=20,ScaleX=125,ScaleY=80";
        let values = normalized_style(style).unwrap();
        assert_eq!(
            [2, 13, 16, 17].map(|i| values[i].as_str()),
            ["67.5", "5.625", "7.5", "3.75"]
        );
        assert_eq!(
            [19, 20, 21].map(|i| values[i].as_str()),
            ["100", "110", "75"]
        );
        assert_eq!([11, 12].map(|i| values[i].as_str()), ["125", "80"]);
        let ass = srt_to_ass(
            &srt("<font size=24>A<font size=12>B</font>C</font>D"),
            style,
        )
        .unwrap();
        assert!(ass.contains("PlayResX: 1920\nPlayResY: 1080"));
        assert!(ass.contains("{\\fs90}A{\\fs45}B{\\fs90}C{\\fs}D"));
        assert_eq!(normalized_style("MarginV=1").unwrap()[21], "4");
        assert!(normalized_style("FontSize=1000000").is_err());
        assert!(cue_text("<font size=1000000>x</font>").is_err());
    }

    #[test]
    fn every_style_field_and_alpha() {
        let style = "Name=Custom,Fontname=O'Brien Sans,Fontsize=18.5,PrimaryColour=&H80112233,SecondaryColour=#FF0000,OutlineColour=&habcdef&,BackColour=-1,Bold=-1,Italic=1,Underline=0,StrikeOut=-1,ScaleX=90,ScaleY=110,Spacing=-2.5,Angle=45,BorderStyle=3,Outline=2.5,Shadow=1,Alignment=6,MarginL=3,MarginR=4,MarginV=5,Encoding=1";
        assert_eq!(style_values(style).unwrap().join(","), "Custom,O'Brien Sans,18.5,&H80112233,&H000000FF,&H00ABCDEF,&HFFFFFFFF,-1,1,0,-1,90,110,-2.5,45,3,2.5,1,8,3,4,5,1");
    }

    #[test]
    fn legacy_alignment_mapping() {
        for (legacy, ass) in [
            (1, 1),
            (2, 2),
            (3, 3),
            (5, 7),
            (6, 8),
            (7, 9),
            (9, 4),
            (10, 5),
            (11, 6),
        ] {
            assert_eq!(
                style_values(&format!("Alignment={legacy}")).unwrap()[18],
                ass.to_string()
            );
        }
        for invalid in [0, 4, 8, 12, -1] {
            assert!(style_values(&format!("Alignment={invalid}")).is_err());
        }
    }

    #[test]
    fn invalid_style_fields_fail_explicitly() {
        for style in [
            "Unknown=1",
            "Bold=1,bold=0",
            "Fontname=x\\y",
            "Fontname={x}",
            "Fontname=",
            "Fontsize=0",
            "Fontsize=1000001",
            "Fontsize=NaN",
            "Fontsize=1e2",
            "Bold=2",
            "MarginL=1.5",
            "MarginV=-1",
            "BorderStyle=2",
            "PrimaryColour=blue",
            "Outline=-1",
            "ScaleX=0",
            "Encoding=-1",
            "Bold=1,",
            "Fontname=A\n[Events]",
        ] {
            assert!(style_values(style).is_err(), "{style}");
        }
    }

    #[test]
    fn nested_font_restores_every_attribute() {
        let text = "<font color='red' face=Arial size=20>A<font color=#0f0 face='Other Sans' size=30>B</font>C</font>D";
        assert_eq!(cue_text(text).unwrap(), "{\\c&H0000FF&}{\\fnArial}{\\fs75}A{\\c&H00FF00&}{\\fnOther Sans}{\\fs112.5}B{\\c&H0000FF&}{\\fnArial}{\\fs75}C{\\c}{\\fn}{\\fs}D");
        assert_eq!(
            cue_text("<font color=orange>A<font size=12>B</font>C</font>").unwrap(),
            "{\\c&H00A5FF&}A{\\fs45}B{\\fs}C{\\c}"
        );
    }

    #[test]
    fn rich_text_nesting_and_line_breaks() {
        assert_eq!(
            cue_text("<b>A<b>B</b>C</b><i>I</i><u>U</u><s>S</s><br/>x\r\ny\rz").unwrap(),
            "{\\b1}A{\\b1}B{\\b1}C{\\b0}{\\i1}I{\\i0}{\\u1}U{\\u0}{\\s1}S{\\s0}\\Nx\\Ny\\Nz"
        );
    }

    #[test]
    fn literals_emoji_unknown_tags_and_control_characters() {
        assert_eq!(
            cue_text("你好😀 {\\pos(0,0)} \\N <em>it's \"quoted\"</em>\u{0}\t").unwrap(),
            "你好😀 \\{\\\u{2060}pos(0,0)\\} \\\u{2060}N <em>it's \"quoted\"</em>\t"
        );
    }

    #[test]
    fn unsupported_recognized_effects_fail() {
        for text in [
            "<font color=invalid>x</font>",
            "<font style='bold'>x</font>",
            "<font size=0>x</font>",
            "<font size='+2'>x</font>",
            "<font size=1000001>x</font>",
            "<font color=red COLOR=blue>x</font>",
            "<font face='A\\B'>x</font>",
            "<font size='12'face='Arial'>x</font>",
            "<b class=x>x</b>",
            "<br style=x>",
            "<font size>x</font>",
            "</font color=red>",
        ] {
            assert!(cue_text(text).is_err(), "{text}");
        }
    }

    #[test]
    fn all_named_colors_are_supported() {
        for name in [
            "black", "silver", "gray", "white", "maroon", "red", "purple", "fuchsia", "green",
            "lime", "olive", "yellow", "navy", "blue", "teal", "aqua", "orange",
        ] {
            assert!(html_color(name).is_ok(), "{name}");
        }
        assert_eq!(html_color("#abc").unwrap(), "CCBBAA");
        assert_eq!(html_color(" RED ").unwrap(), "0000FF");
    }

    #[test]
    fn times_round_carry_sort_and_omit_collapsed_cues() {
        let text = "8\n00:00:59,995 --> 00:01:01,005\nlater\n\n99\n00:00:00,005 --> 00:00:00,015\nfirst\n\n1\n00:00:00,005 --> 00:00:00,020\nsecond\n\n2\n00:00:00,001 --> 00:00:00,004\nomitted";
        let output = srt_to_ass(text, "").unwrap();
        let lines: Vec<_> = output
            .lines()
            .filter(|l| l.starts_with("Dialogue:"))
            .collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("0:00:00.01,0:00:00.02") && lines[0].ends_with("first"));
        assert!(lines[1].ends_with("second"));
        assert!(lines[2].contains("0:01:00.00,0:01:01.01"));
    }

    #[test]
    fn malformed_and_out_of_bounds_times_fail() {
        for text in [
            "",
            "bad",
            "1\n00:00:01,000",
            "1\n00:00:00,000 --> 00:00:00,000\nx",
            "1\n00:00:01,000 --> 00:00:00,000\nx",
            "1\n00:60:00,000 --> 01:00:01,000\nx",
            "1\n00:00:60,000 --> 01:00:01,000\nx",
            "1\n00:00:00,1000 --> 01:00:01,000\nx",
            "1\n18446744073709551615:00:00,000 --> 18446744073709551615:00:01,000\nx",
        ] {
            assert!(srt_to_ass(text, "").is_err(), "{text}");
        }
        assert!(centiseconds(u64::MAX).is_err());
        assert_eq!(ass_time(360_000), "1:00:00.00");
        assert!(srt_to_ass(
            &format!("\u{feff}{}", srt("CRLF").replace('\n', "\r\n")),
            ""
        )
        .is_ok());
    }

    #[test]
    fn fonts_are_shallow_sorted_bounded_and_optional() {
        assert!(font_paths(None).unwrap().is_empty());
        let dir = std::env::temp_dir().join(format!(
            "subtitle-native-fonts-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(dir.clone());
        for name in ["z.TTF", "a.otf", "b.ttc", "c.otc", "ignored.txt"] {
            std::fs::write(dir.join(name), []).unwrap();
        }
        std::fs::create_dir(dir.join("nested")).unwrap();
        std::fs::write(dir.join("nested/deep.ttf"), []).unwrap();
        assert_eq!(
            font_paths(Some(&dir)).unwrap(),
            ["a.otf", "b.ttc", "c.otc", "z.TTF"].map(|s| dir.join(s))
        );
        assert!(font_paths(Some(&dir.join("missing"))).is_err());
        assert!(font_paths(Some(&dir.join("a.otf"))).is_err());
        for i in 0..125 {
            std::fs::write(dir.join(format!("extra{i}.ttf")), []).unwrap();
        }
        assert!(font_paths(Some(&dir)).unwrap_err().contains("128"));
    }
}
