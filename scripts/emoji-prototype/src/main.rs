//! Isolated CPU Skia/SkParagraph experiment, not an ASS replacement.
use skia_safe::{
    color_filters, image_filters, surfaces,
    textlayout::{
        FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, TextStyle,
        TypefaceFontProvider,
    },
    AlphaType, BlendMode, Color, ColorType, EncodedImageFormat, FontMgr, Image, ImageInfo, Paint,
    Rect, Surface,
};
use std::{
    collections::HashSet,
    error::Error,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::Instant,
};
use unicode_segmentation::UnicodeSegmentation;

const PAD: f32 = 16.0;
const WIDTH: i32 = 1920;
const HEIGHT: i32 = 1080;
const FPS: usize = 60;
const FRAMES: usize = 360;
type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct Renderer {
    fonts: FontCollection,
    emoji: HashSet<String>,
    qualified: Vec<String>,
}

fn parse_emoji(data: &str) -> (HashSet<String>, Vec<String>) {
    let mut all = HashSet::new();
    let mut qualified = Vec::new();
    for line in data.lines() {
        let Some((codes, status)) = line.split('#').next().unwrap_or("").split_once(';') else {
            continue;
        };
        let sequence: String = codes
            .split_whitespace()
            .map(|code| {
                char::from_u32(u32::from_str_radix(code, 16).expect("Unicode fixture code point"))
                    .expect("Unicode scalar")
            })
            .collect();
        if status.trim() == "fully-qualified" {
            qualified.push(sequence.clone());
        }
        all.insert(sequence);
    }
    (all, qualified)
}

impl Renderer {
    fn new(assets: &Path) -> Result<Self> {
        let manager = FontMgr::new();
        let mut provider = TypefaceFontProvider::new();
        for (file, alias) in [
            ("NotoSansCJKsc-Regular.otf", "Body"),
            ("Noto-COLRv1.ttf", "Emoji"),
        ] {
            let bytes = fs::read(assets.join(file))?;
            let face = manager
                .new_from_data(&bytes, None)
                .ok_or_else(|| format!("Cannot load {file} with this platform's font backend"))?;
            provider.register_typeface(face, alias);
        }
        let mut fonts = FontCollection::new();
        fonts.set_asset_font_manager(Some(provider.into()));
        // A successful result must not secretly depend on installed system fonts.
        fonts.disable_font_fallback();
        let (emoji, qualified) = parse_emoji(&fs::read_to_string(assets.join("emoji-test.txt"))?);
        if qualified.is_empty() {
            return Err("Empty Unicode emoji fixture".into());
        }
        Ok(Self {
            fonts,
            emoji,
            qualified,
        })
    }

    fn paragraph(&self, text: &str, size: f32, width: f32, color: Color) -> Paragraph {
        let mut body = TextStyle::new();
        body.set_font_families(&["Body"])
            .set_font_size(size)
            .set_color(color);
        let mut emoji = body.clone();
        emoji.set_font_families(&["Emoji"]);
        let mut style = ParagraphStyle::new();
        style.set_text_style(&body);
        let mut builder = ParagraphBuilder::new(&style, self.fonts.clone());
        // Select the font for an entire grapheme, then merge adjacent same-font
        // graphemes into runs. This preserves shaping across ordinary letters.
        let mut run = String::new();
        let mut previous = false;
        for grapheme in text.graphemes(true) {
            let is_emoji = self.emoji.contains(grapheme);
            if !run.is_empty() && is_emoji != previous {
                builder
                    .push_style(if previous { &emoji } else { &body })
                    .add_text(&run)
                    .pop();
                run.clear();
            }
            run.push_str(grapheme);
            previous = is_emoji;
        }
        if !run.is_empty() {
            builder
                .push_style(if previous { &emoji } else { &body })
                .add_text(&run)
                .pop();
        }
        let mut paragraph = builder.build();
        paragraph.layout(width);
        paragraph
    }

    fn sprite(&self, text: &str, size: f32, width: f32, color: Color) -> Result<Image> {
        let mut paragraph = self.paragraph(text, size, width, color);
        if paragraph.unresolved_glyphs() != Some(0) {
            return Err(format!(
                "Unresolved glyphs in {text:?}: {:?}",
                paragraph.unresolved_codepoints()
            )
            .into());
        }
        let w = (paragraph.longest_line().ceil() + 2.0 * PAD) as i32;
        let h = (paragraph.height().ceil() + 2.0 * PAD) as i32;
        let mut content = surface(w, h)?;
        paragraph.paint(content.canvas(), (PAD, PAD));
        let image = content.image_snapshot();
        let mut styled = surface(w, h)?;
        // Build outline/shadow from the combined alpha mask. Never tint the
        // foreground emoji, and apply whole-message opacity only at compositing.
        let mut shadow = Paint::default();
        shadow.set_color_filter(color_filters::blend(Color::BLACK, BlendMode::SrcIn));
        shadow.set_image_filter(image_filters::blur((2.0, 2.0), None, None, None));
        shadow.set_alpha_f(0.7);
        styled
            .canvas()
            .draw_image(&image, (2.0, 3.0), Some(&shadow));
        let mut outline = Paint::default();
        outline.set_color_filter(color_filters::blend(Color::BLACK, BlendMode::SrcIn));
        outline.set_image_filter(image_filters::dilate((1.5, 1.5), None, None));
        styled
            .canvas()
            .draw_image(&image, (0.0, 0.0), Some(&outline));
        styled.canvas().draw_image(&image, (0.0, 0.0), None);
        Ok(styled.image_snapshot())
    }
}

fn surface(width: i32, height: i32) -> Result<Surface> {
    if width <= 0 || height <= 0 || width > 8192 || height > 8192 {
        return Err(format!("Unsafe raster dimensions {width}x{height}").into());
    }
    let mut result =
        surfaces::raster_n32_premul((width, height)).ok_or("Cannot allocate raster surface")?;
    result.canvas().clear(Color::TRANSPARENT);
    Ok(result)
}

fn save(image: &Image, path: &Path) -> Result<()> {
    let png = image
        .encode(None, EncodedImageFormat::PNG, None)
        .ok_or("PNG encoding failed")?;
    fs::write(path, png.as_bytes())?;
    Ok(())
}

fn rgba(surface: &mut Surface, pixels: &mut [u8]) -> Result<()> {
    // Skia's native pixels are premultiplied and may be BGRA. The FFmpeg pipe
    // contract is explicitly straight/unpremultiplied RGBA, not native memory.
    let info = ImageInfo::new(
        (surface.width(), surface.height()),
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        None,
    );
    if !surface.read_pixels(&info, pixels, surface.width() as usize * 4, (0, 0)) {
        return Err("RGBA readback failed".into());
    }
    Ok(())
}

fn fixtures(renderer: &Renderer, out: &Path, load_ms: f64) -> Result<()> {
    let samples = [
        "中文 English：😀 😂 🥰 👍 🎉 🔥",
        "肤色与职业：👍🏻 👍🏽 👍🏿 👩🏽‍💻 🧑‍🚀",
        "组合与旗帜：👨‍👩‍👧‍👦 🏳️‍🌈 🇨🇳 🇺🇸",
        "键帽与变体：1️⃣ #️⃣ *️⃣ ❤️ ☺️ ©️ / 1 # * ©︎",
        "正文仍有颜色 Hello，emoji 保留原色 🥰🎉",
        "自动换行：这是一段中文与 English 混排字幕 👩🏽‍💻，一家人 👨‍👩‍👧‍👦 开心出游 🇨🇳！",
    ];
    let mut sheet = surface(1600, 880)?;
    let mut box_paint = Paint::default();
    box_paint.set_color(Color::from_rgb(30, 34, 42));
    sheet
        .canvas()
        .draw_rect(Rect::from_xywh(0.0, 0.0, 800.0, 880.0), &box_paint);
    box_paint.set_color(Color::from_rgb(245, 245, 245));
    sheet
        .canvas()
        .draw_rect(Rect::from_xywh(800.0, 0.0, 800.0, 880.0), &box_paint);
    let mut times = Vec::new();
    for (i, text) in samples.iter().enumerate() {
        let start = Instant::now();
        let color = if i == 4 {
            Color::from_rgb(80, 210, 255)
        } else {
            Color::WHITE
        };
        let sprite = renderer.sprite(text, 32.0, 740.0, color)?;
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        save(&sprite, &out.join(format!("sample-{i}.png")))?;
        if i == 0 {
            // Exercise exactly the same readback contract used by the video pipe.
            let mut raw = surface(sprite.width(), sprite.height())?;
            raw.canvas().draw_image(&sprite, (0.0, 0.0), None);
            let mut bytes = vec![0; sprite.width() as usize * sprite.height() as usize * 4];
            rgba(&mut raw, &mut bytes)?;
            fs::write(out.join("sample-0.rgba"), bytes)?;
        }
        for x in [12.0, 812.0] {
            sheet
                .canvas()
                .draw_image(&sprite, (x, 12.0 + i as f32 * 124.0), None);
        }
    }
    // Same sprite with opacity 0, 0.8 and 1; nothing should be painted for 0.
    let opacity_sprite = renderer.sprite("透明度 😀👍🏽", 32.0, 400.0, Color::WHITE)?;
    for (i, opacity) in [0.0, 0.8, 1.0].into_iter().enumerate() {
        let mut paint = Paint::default();
        paint.set_alpha_f(opacity);
        for x in [0.0, 800.0] {
            sheet.canvas().draw_image(
                &opacity_sprite,
                (x + 10.0 + i as f32 * 260.0, 790.0),
                Some(&paint),
            );
        }
    }
    save(&sheet.image_snapshot(), &out.join("contact-sheet.png"))?;

    // Coverage is a shaping check, not proof of visual fidelity for every emoji.
    let audit_start = Instant::now();
    let mut failures = Vec::new();
    for text in &renderer.qualified {
        let mut p = renderer.paragraph(text, 32.0, 300.0, Color::WHITE);
        let mut glyphs = 0;
        p.visit(|_, info| {
            if let Some(info) = info {
                glyphs += info.glyphs().len();
            }
        });
        if p.unresolved_glyphs() != Some(0) || glyphs != 1 {
            failures.push(serde_json::json!({"emoji": text, "glyphs": glyphs, "unresolved": p.unresolved_glyphs()}));
        }
    }
    let audit_ms = audit_start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    for i in 0..200 {
        // Discard each image: measures distinct rasterization, not a cache hit.
        renderer.sprite(
            &format!("不同弹幕 {i} 中文 Hello 👩🏽‍💻👍🏽🎉"),
            36.0,
            1500.0,
            Color::WHITE,
        )?;
    }
    let distinct_ms = start.elapsed().as_secs_f64() * 1000.0 / 200.0;
    let start = Instant::now();
    for _ in 0..100 {
        renderer.sprite(samples[0], 36.0, 1500.0, Color::WHITE)?;
    }
    let repeated_ms = start.elapsed().as_secs_f64() * 1000.0 / 100.0;

    // An unavailable scalar is observable, not silently replaced by a system font.
    let mut missing = renderer.paragraph("中文\u{10FFFF}😀", 32.0, 800.0, Color::WHITE);
    let missing_detected = missing.unresolved_glyphs().is_some_and(|n| n > 0);
    let wrapped = renderer
        .paragraph(samples[5], 32.0, 740.0, Color::WHITE)
        .line_number();
    let report = serde_json::json!({
        "font_load_ms": load_ms, "sample_raster_ms": times,
        "distinct_raster_mean_ms": distinct_ms, "repeated_raster_mean_ms": repeated_ms,
        "unicode_version": "16.0", "qualified_sequences": renderer.qualified.len(),
        "audit_ms": audit_ms, "shape_failures": failures,
        "missing_glyph_detected": missing_detected, "wrapped_sample_lines": wrapped,
        "system_font_fallback": false,
        "note": "CPU raster; repeated render includes shaping cache but no sprite cache; audit checks glyph count, not all artwork"
    });
    fs::write(
        out.join("fixtures.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    if !failures.is_empty() || !missing_detected || wrapped < 2 {
        return Err("Fixture assertions failed; see fixtures.json".into());
    }
    Ok(())
}

fn stream(renderer: &Renderer, out: &Path, count: usize) -> Result<()> {
    if count == 0 || count > 100 {
        return Err("Concurrent sprite count must be 1..100".into());
    }
    let prepare_start = Instant::now();
    let mut sprites = Vec::new();
    for i in 0..count {
        sprites.push(renderer.sprite(
            &format!("弹幕 {i:02} 中文 Hello 😀 👩🏽‍💻 👍🏽 🎉"),
            32.0,
            1500.0,
            Color::WHITE,
        )?);
    }
    let subtitles = [
        renderer.sprite(
            "彩色字幕：今天好开心 🥰！一家人 👨‍👩‍👧‍👦 出游 🇨🇳",
            48.0,
            1700.0,
            Color::WHITE,
        )?,
        renderer.sprite(
            "第二条字幕：肤色 👍🏻 👍🏽 👍🏿，键帽 1️⃣，爱心 ❤️",
            48.0,
            1700.0,
            Color::from_rgb(80, 210, 255),
        )?,
    ];
    let cache_bytes: usize = sprites
        .iter()
        .chain(subtitles.iter())
        .map(|i| i.width() as usize * i.height() as usize * 4)
        .sum();
    let prepare_ms = prepare_start.elapsed().as_secs_f64() * 1000.0;
    let mut frame = surface(WIDTH, HEIGHT)?;
    let mut pixels = vec![0_u8; WIDTH as usize * HEIGHT as usize * 4];
    let mut stdout = io::stdout().lock();
    let mut paint = Paint::default();
    paint.set_alpha_f(0.8);
    let start = Instant::now();
    let mut compose_ms = Vec::with_capacity(FRAMES);
    let mut pipe_ms = 0.0;
    for n in 0..FRAMES {
        let tick = Instant::now();
        let t = n as f32 / FPS as f32;
        frame.canvas().clear(Color::TRANSPARENT);
        // Bounded fixed sprite set; only positions/opacity change per frame.
        // This is a compositor stress scene, NOT the app's collision scheduler.
        for (i, image) in sprites.iter().enumerate() {
            let travel = WIDTH as f32 + image.width() as f32;
            let phase = (t * (210.0 + (i % 4) as f32 * 25.0) + i as f32 * 137.0) % travel;
            frame.canvas().draw_image(
                image,
                (WIDTH as f32 - phase, 24.0 + (i % 12) as f32 * 66.0),
                Some(&paint),
            );
        }
        // Deliberate empty interval verifies subtitle end-time handling.
        let subtitle = if t < 2.5 {
            Some(&subtitles[0])
        } else if t >= 3.0 {
            Some(&subtitles[1])
        } else {
            None
        };
        if let Some(image) = subtitle {
            frame.canvas().draw_image(
                image,
                (
                    (WIDTH - image.width()) as f32 / 2.0,
                    (HEIGHT - image.height() - 44) as f32,
                ),
                None,
            );
        }
        rgba(&mut frame, &mut pixels)?;
        compose_ms.push(tick.elapsed().as_secs_f64() * 1000.0);
        if n == 120 {
            save(
                &frame.image_snapshot(),
                &out.join(format!("overlay-{count}.png")),
            )?;
        }
        let write = Instant::now();
        stdout.write_all(&pixels)?;
        pipe_ms += write.elapsed().as_secs_f64() * 1000.0;
    }
    stdout.flush()?;
    let elapsed = start.elapsed().as_secs_f64();
    compose_ms.sort_by(f64::total_cmp);
    let mean = compose_ms.iter().sum::<f64>() / FRAMES as f64;
    fs::write(
        out.join(format!("stream-{count}.json")),
        serde_json::to_vec_pretty(&serde_json::json!({
            "width": WIDTH, "height": HEIGHT, "fps": FPS, "frames": FRAMES,
            "concurrent_sprites": count, "sprite_cache_bytes": cache_bytes,
            "prepare_ms": prepare_ms, "compose_and_rgba_mean_ms": mean,
            "compose_and_rgba_p95_ms": compose_ms[FRAMES * 95 / 100],
            "pipe_wait_and_copy_ms": pipe_ms, "stream_seconds": elapsed,
            "producer_throughput_fps": FRAMES as f64 / elapsed,
            "raw_bytes_per_second_at_60fps": WIDTH as u64 * HEIGHT as u64 * 4 * FPS as u64,
            "alpha_contract": "RGBA8888 unpremultiplied -> FFmpeg overlay alpha=straight"
        }))?,
    )?;
    Ok(())
}

fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = root.join("output");
    fs::create_dir_all(&out)?;
    let start = Instant::now();
    let renderer = Renderer::new(&root.join("assets"))?;
    let load_ms = start.elapsed().as_secs_f64() * 1000.0;
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("fixtures") => fixtures(&renderer, &out, load_ms),
        Some("stream") => stream(
            &renderer,
            &out,
            args.get(2).map(|s| s.parse()).transpose()?.unwrap_or(24),
        ),
        _ => Err("Usage: color-emoji-prototype fixtures | stream [concurrent-sprites]".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_sequence_parser_preserves_zwj_and_modifiers() {
        let (all, qualified) = parse_emoji("# header\n1F469 1F3FD 200D 1F4BB ; fully-qualified # test\n2764 ; unqualified # heart\n");
        assert!(all.contains("👩🏽‍💻"));
        assert!(all.contains("❤"));
        assert_eq!(qualified, vec!["👩🏽‍💻"]);
    }
    #[test]
    fn segmentation_keeps_sequences_and_plain_digits_separate() {
        assert_eq!(
            "A1👩🏽‍💻🇨🇳1️⃣❤️".graphemes(true).collect::<Vec<_>>(),
            vec!["A", "1", "👩🏽‍💻", "🇨🇳", "1️⃣", "❤️"]
        );
    }
    #[test]
    fn rejects_unbounded_surface_allocations() {
        assert!(surface(9000, 100).is_err());
        assert!(surface(0, 100).is_err());
    }
}
