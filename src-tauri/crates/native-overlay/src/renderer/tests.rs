use super::*;
use crate::AssSource;

const FIELDS: &str = "Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding";
const STYLE: &str = "Default,Arial,64,&H00FFFFFF,&H00FFFFFF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,3,2,2,100,100,60,1";

fn input(text: &str, style: &str) -> RenderInput {
    RenderInput { documents: vec![AssSource { kind: SourceKind::Subtitle, text: format!("[Script Info]\nScriptType: v4.00+\nPlayResX: 1920\nPlayResY: 1080\n[V4+ Styles]\nFormat: {FIELDS}\nStyle: {style}\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,{text}\n") }], images: vec![] }
}

/// Each fixture has immutable, identical artwork at a distinct path.
struct TestImage(PathBuf);

impl TestImage {
    fn new() -> Self {
        let mut s = surface(8, 4, CACHE_BYTES).unwrap();
        s.canvas().clear(Color::RED);
        let mut paint = Paint::default();
        paint.set_color(Color::from_argb(128, 0, 255, 0));
        paint.set_blend_mode(BlendMode::Src);
        s.canvas()
            .draw_rect(Rect::from_xywh(4.0, 0.0, 4.0, 4.0), &paint);
        Self::from_surface(s)
    }

    fn from_surface(mut s: Surface) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "native-overlay-cache-{}-{}.png",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(
            &path,
            s.image_snapshot()
                .encode(None, EncodedImageFormat::PNG, None)
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        Self(path)
    }

    fn event(&self) -> ImageEvent {
        ImageEvent {
            path: self.0.clone(),
            start: 1.0,
            end: 3.0,
            from: (0.0, 0.0),
            to: (960.0, 540.0),
            width: 320.0,
            height: 160.0,
            opacity: 0.5,
        }
    }
}

impl Drop for TestImage {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn timings(stats: &RenderStats) -> [Duration; 5] {
    [
        stats.text_layout,
        stats.text_raster,
        stats.image_prepare,
        stats.composition,
        stats.readback,
    ]
}

fn counters(stats: &RenderStats) -> [u64; 5] {
    [
        stats.frames,
        stats.text_cache_hits,
        stats.text_cache_misses,
        stats.image_cache_hits,
        stats.image_cache_misses,
    ]
}

fn bbox(data: &[u8], w: usize) -> Option<(usize, usize, usize, usize)> {
    let mut r = (usize::MAX, usize::MAX, 0, 0);
    for (i, p) in data.chunks_exact(4).enumerate() {
        if p[3] > 8 {
            r.0 = r.0.min(i % w);
            r.1 = r.1.min(i / w);
            r.2 = r.2.max(i % w);
            r.3 = r.3.max(i / w);
        }
    }
    (r.0 != usize::MAX).then_some(r)
}

#[test]
fn inline_font_size_changes_raster_and_cluster_remains_whole() {
    let mut normal = Renderer::new(960, 540, input("😀", STYLE), vec![]).unwrap();
    let normal = bbox(&normal.frame(1.0).unwrap(), 960).unwrap();
    let mut larger = Renderer::new(960, 540, input(r"{\fnArial\fs128}😀", STYLE), vec![]).unwrap();
    let larger = bbox(&larger.frame(1.0).unwrap(), 960).unwrap();
    assert!(larger.2 - larger.0 > (normal.2 - normal.0) * 3 / 2);
    let fonts = Fonts::new(vec![]).unwrap();
    let event = ass::parse(
        &input(r"👩{\b1}🏽‍💻", STYLE).documents[0].text,
        SourceKind::Subtitle,
    )
    .unwrap()
    .remove(0);
    let mut paragraph = fonts.paragraph(&event, false);
    paragraph.layout(1000.0);
    let mut count = 0;
    paragraph.visit(|_, info| {
        if let Some(info) = info {
            count += info.glyphs().len();
        }
    });
    assert_eq!(count, 1);
}

#[test]
fn color_emoji_raster_and_rgba_png_parity() {
    let mut r = Renderer::new(960, 540, input("😀 👩🏽‍💻 👨‍👩‍👧‍👦 🇨🇳 1️⃣ ❤️", STYLE), vec![]).unwrap();
    let pixels = r.frame(1.0).unwrap();
    assert_eq!(pixels.len(), 960 * 540 * 4);
    let mut reused = vec![0xa5; pixels.len()];
    r.frame_into(1.0, &mut reused).unwrap();
    assert_eq!(reused, pixels);
    assert_eq!(r.missing_glyph_count(), 0);
    let colored = pixels
        .chunks_exact(4)
        .filter(|p| p[3] > 200 && p[..3].iter().max().unwrap() - p[..3].iter().min().unwrap() > 50)
        .count();
    assert!(colored > 500, "missing color emoji: {colored} pixels");
    let png = r.png(1.0).unwrap();
    let image = Image::from_encoded(Data::new_copy(&png)).unwrap();
    let mut decoded = vec![0; pixels.len()];
    let info = ImageInfo::new((960, 540), ColorType::RGBA8888, AlphaType::Unpremul, None);
    assert!(image.read_pixels(
        &info,
        &mut decoded,
        960 * 4,
        (0, 0),
        skia_safe::image::CachingHint::Disallow
    ));
    assert!(pixels.iter().zip(decoded).all(|(a, b)| a.abs_diff(b) <= 1));
    assert!(r.frame(0.99).unwrap().iter().all(|v| *v == 0));
    assert!(r.frame(3.0).unwrap().iter().all(|v| *v == 0));
    assert_eq!(r.frame(1.0).unwrap(), pixels);
}

#[test]
fn emoji_alpha_and_missing_glyph_are_nonfatal() {
    let style = STYLE
        .replace("&H00FFFFFF", "&H80FFFFFF")
        .replace("&H00000000", "&H80000000");
    let mut r = Renderer::new(960, 540, input("😀", &style), vec![]).unwrap();
    let data = r.frame(1.0).unwrap();
    let max = data.chunks_exact(4).map(|p| p[3]).max().unwrap();
    assert!(
        (120..=135).contains(&max),
        "fill/outline incorrectly multiply alpha: {max}"
    );
    let mut r = Renderer::new(960, 540, input("\u{10ffff}", STYLE), vec![]).unwrap();
    // macOS LastResort can claim every Unicode scalar; disable only fallback
    // here to exercise the real unresolved-glyph path deterministically.
    r.fonts.collection.disable_font_fallback();
    let data = r.frame(1.0).unwrap();
    assert!(r.missing_glyph_count() > 0);
    assert!(
        bbox(&data, 960).is_some(),
        "missing glyph must remain visible"
    );
}

#[test]
fn unicode16_qualified_graphemes_shape_as_one_glyph() {
    let fonts = Fonts::new(vec![]).unwrap();
    let mut event = ass::parse(&input("😀", STYLE).documents[0].text, SourceKind::Subtitle)
        .unwrap()
        .remove(0);
    let mut count = 0;
    for line in include_str!("../../assets/emoji-test.txt").lines() {
        let Some((codes, status)) = line.split('#').next().unwrap_or("").split_once(';') else {
            continue;
        };
        if status.trim() != "fully-qualified" {
            continue;
        }
        let text: String = codes
            .split_whitespace()
            .map(|c| char::from_u32(u32::from_str_radix(c, 16).unwrap()).unwrap())
            .collect();
        event.runs[0].text = text.clone();
        let mut p = fonts.paragraph(&event, false);
        p.layout(1000.0);
        assert_eq!(p.unresolved_glyphs(), Some(0), "{text}");
        let mut glyphs = 0;
        p.visit(|_, info| {
            if let Some(info) = info {
                glyphs += info.glyphs().len();
            }
        });
        assert_eq!(glyphs, 1, "{text}");
        count += 1;
    }
    assert!(count > 3500);
}

#[test]
fn collision_and_alignment_layout() {
    let first = Rect::from_xywh(10.0, 100.0, 100.0, 30.0);
    let mut second = first;
    assert_eq!(stack(&mut second, 0, 2, &[(0, 2, first)]), -32.0);
    assert!(!second.intersects(first));
    let mut top = first;
    assert_eq!(stack(&mut top, 0, 8, &[(0, 8, first)]), 32.0);
    let mut other_layer = first;
    assert_eq!(stack(&mut other_layer, 1, 2, &[(0, 2, first)]), 0.0);
    let one = input("test", STYLE);
    let mut two = one.clone();
    two.documents[0]
        .text
        .push_str("Dialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,test\n");
    let a = bbox(
        &Renderer::new(960, 540, one, vec![])
            .unwrap()
            .frame(2.0)
            .unwrap(),
        960,
    )
    .unwrap();
    let b = bbox(
        &Renderer::new(960, 540, two, vec![])
            .unwrap()
            .frame(2.0)
            .unwrap(),
        960,
    )
    .unwrap();
    assert!(b.1 < a.1 && b.3 == a.3);
}

#[test]
fn safe_allocation_and_invalid_input_bounds() {
    assert!(Renderer::new(0, 1080, RenderInput::default(), vec![]).is_err());
    assert!(Renderer::new(u32::MAX, 1080, RenderInput::default(), vec![]).is_err());
    assert!(raster_bytes(16384.0, 16384.0, MAX_FRAME_BYTES).is_err());
    let mut r = Renderer::new(64, 64, RenderInput::default(), vec![]).unwrap();
    assert!(r.frame(f64::NAN).is_err());
    assert_eq!(r.frame(0.0).unwrap(), vec![0; 64 * 64 * 4]);
}

#[test]
fn image_artwork_is_contained_and_centered_in_reserved_footprint() {
    let path = std::env::temp_dir().join(format!(
        "native-overlay-image-contain-{}.png",
        std::process::id()
    ));
    let mut s = surface(16, 4, CACHE_BYTES).unwrap();
    s.canvas().clear(Color::RED);
    fs::write(
        &path,
        s.image_snapshot()
            .encode(None, EncodedImageFormat::PNG, None)
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    let input = RenderInput {
        documents: vec![],
        images: vec![ImageEvent {
            path: path.clone(),
            start: 0.0,
            end: 1.0,
            from: (0.0, 0.0),
            to: (0.0, 0.0),
            width: 320.0,
            height: 160.0,
            opacity: 1.0,
        }],
    };
    let mut r = Renderer::new(192, 108, input, vec![]).unwrap();
    let sprite = r
        .image_sprite(0, false, &mut super::SurfaceFactory::raster())
        .unwrap();
    assert_eq!(sprite.size, (32.0, 16.0));
    assert_eq!((sprite.image.width(), sprite.image.height()), (32, 16));
    assert_eq!(bbox(&r.frame(0.0).unwrap(), 192), Some((0, 4, 31, 11)));
    fs::remove_file(path).unwrap();
}

#[test]
fn frame_into_rejects_wrong_lengths_without_changing_render_state() {
    check_invalid_frame_lengths(Renderer::frame_into, "RGBA");
    check_invalid_frame_lengths(Renderer::frame_premultiplied_bgra_into, "BGRA");
}

type FrameWriter = fn(&mut Renderer, f64, &mut [u8]) -> Result<(), String>;

fn check_invalid_frame_lengths(write: FrameWriter, format: &str) {
    let image = TestImage::new();
    let mut source = input("😀", STYLE);
    source.images.push(image.event());
    let mut r = Renderer::new(192, 108, source, vec![]).unwrap();
    r.set_profiling(true);
    let expected = 192 * 108 * 4;
    // First reject an active timestamp: no cache preparation or frame is counted.
    let mut short = vec![0x5a; expected - 1];
    assert!(write(&mut r, 1.0, &mut short).is_err());
    assert!(short.iter().all(|byte| *byte == 0x5a));
    assert_eq!(counters(&r.stats()), [0; 5]);
    assert_eq!(timings(&r.stats()), [Duration::ZERO; 5]);
    assert!(r.active_text.is_empty() && r.active_images.is_empty());

    let pixels = r.frame(1.0).unwrap();
    let before = r.stats();
    let active_text = r.active_text.clone();
    let active_images = r.active_images.clone();
    let placed = r.placed.clone();
    for (time, length) in [(10.0, 0), (0.0, expected - 1), (f64::NAN, expected + 1)] {
        let mut wrong = vec![0x5a; length];
        let error = write(&mut r, time, &mut wrong).unwrap_err();
        assert!(
            error.contains(&format!("{format} buffer length")),
            "{error}"
        );
        assert!(wrong.iter().all(|byte| *byte == 0x5a));
        assert_eq!(counters(&r.stats()), counters(&before));
        assert_eq!(timings(&r.stats()), timings(&before));
        assert_eq!(r.active_text, active_text);
        assert_eq!(r.active_images, active_images);
        assert_eq!(r.placed, placed);
        let mut unchanged = vec![0; expected];
        let info = ImageInfo::new((192, 108), ColorType::RGBA8888, AlphaType::Unpremul, None);
        assert!(r
            .surface
            .read_pixels(&info, &mut unchanged, 192 * 4, (0, 0)));
        assert_eq!(unchanged, pixels);
    }
    // The next valid call still observes the correct half-open timeline.
    assert_eq!(r.frame(1.0).unwrap(), pixels);
    assert!(r.frame(3.0).unwrap().iter().all(|byte| *byte == 0));
    assert_eq!(r.frame(1.0).unwrap(), pixels);
    assert_eq!(counters(&r.stats()), [4, 2, 1, 2, 1]);
}

#[test]
fn reused_frame_buffer_matches_allocating_frames_across_seeks_and_transparency() {
    let image = TestImage::new();
    let mut source = input("😀", &STYLE.replace("&H00FFFFFF", "&H80FFFFFF"));
    source.images.push(image.event());
    let mut allocating = Renderer::new(192, 108, source.clone(), vec![]).unwrap();
    let mut reused = Renderer::new(192, 108, source, vec![]).unwrap();
    let mut buffer = vec![0xa5; 192 * 108 * 4];
    let pointer = buffer.as_ptr();
    for time in [0.0, 1.0, 1.5, 2.99, 3.0, 8.0, 1.5, 0.99, 1.0] {
        reused.frame_into(time, &mut buffer).unwrap();
        assert_eq!(buffer.as_ptr(), pointer);
        assert_eq!(buffer, allocating.frame(time).unwrap(), "time {time}");
        if (1.0..3.0).contains(&time) {
            assert!(buffer.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
        } else {
            assert!(buffer.iter().all(|byte| *byte == 0));
        }
    }
    assert_eq!(counters(&reused.stats()), [9, 4, 1, 4, 1]);
    assert_eq!(counters(&reused.stats()), counters(&allocating.stats()));
}

#[test]
fn image_cache_shares_only_paths_and_rounded_output_dimensions() {
    let shared = TestImage::new();
    let other = TestImage::new();
    let copies: Vec<_> = (0..5).map(|_| TestImage::new()).collect();
    let mut first = shared.event();
    first.width = 311.0;
    first.height = 151.0;
    let mut second = first.clone();
    second.start = 1.25;
    second.end = 4.0;
    second.from = (250.0, 100.0);
    second.to = (600.0, 200.0);
    second.opacity = 0.75;
    second.width = 319.0;
    second.height = 159.0;
    let mut different_size = second.clone();
    different_size.width = 321.0;
    let mut different_height = second.clone();
    different_height.height = 161.0;
    let mut different_path = second.clone();
    different_path.path = other.0.clone();
    let source = RenderInput {
        documents: vec![],
        images: vec![
            first,
            second,
            different_size,
            different_height,
            different_path,
        ],
    };
    let mut reference = source.clone();
    for (event, copy) in reference.images.iter_mut().zip(&copies) {
        event.path = copy.0.clone();
    }
    let mut r = Renderer::new(192, 108, source, vec![]).unwrap();
    let mut reference = Renderer::new(192, 108, reference, vec![]).unwrap();
    assert_eq!(r.image_sprite_ids, [0, 0, 1, 2, 3]);
    assert_eq!(r.frame(1.5).unwrap(), reference.frame(1.5).unwrap());
    assert_eq!(counters(&r.stats()), [1, 0, 0, 1, 4]);
    assert_eq!(counters(&reference.stats()), [1, 0, 0, 0, 5]);
    for time in [2.0, 3.0, 4.0, 1.5] {
        assert_eq!(r.frame(time).unwrap(), reference.frame(time).unwrap());
    }
    assert_eq!(counters(&r.stats()), [5, 0, 0, 15, 4]);
    assert_eq!(counters(&reference.stats()), [5, 0, 0, 14, 5]);
}

#[test]
fn evicted_image_redecodes_with_identical_pixels_after_backward_seek() {
    let first = TestImage::new();
    let second = TestImage::new();
    let a = first.event();
    let mut b = second.event();
    b.start = 3.0;
    b.end = 5.0;
    let mut r = Renderer::new(
        192,
        108,
        RenderInput {
            documents: vec![],
            images: vec![a, b],
        },
        vec![],
    )
    .unwrap();
    // One 32 × 16 sprite fits; the next one must evict it.
    r.cache = ByteLru::new(32 * 16 * 4);
    let original = r.frame(1.5).unwrap();
    assert!(bbox(&original, 192).is_some());
    assert_eq!(counters(&r.stats()), [1, 0, 0, 0, 1]);
    r.frame(3.5).unwrap();
    assert_eq!(counters(&r.stats()), [2, 0, 0, 0, 2]);
    assert_eq!(r.frame(1.5).unwrap(), original);
    assert_eq!(counters(&r.stats()), [3, 0, 0, 0, 3]);
    assert_eq!(r.frame(1.5).unwrap(), original);
    assert_eq!(counters(&r.stats()), [4, 0, 0, 1, 3]);
}

#[test]
fn profiling_is_opt_in_cumulative_and_excludes_png_work() {
    check_profiling(Renderer::frame_into);
    check_profiling(Renderer::frame_premultiplied_bgra_into);
}

fn check_profiling(write: FrameWriter) {
    let image = TestImage::new();
    let mut source = input("😀", STYLE);
    source.images.push(image.event());
    let mut r = Renderer::new(192, 108, source, vec![]).unwrap();
    r.frame(1.0).unwrap();
    assert_eq!(timings(&r.stats()), [Duration::ZERO; 5]);
    assert_eq!(counters(&r.stats()), [1, 0, 1, 0, 1]);

    r.set_profiling(true);
    assert_eq!(counters(&r.stats()), [1, 0, 1, 0, 1]);
    // Cold caches exercise all five stages, not just composition/readback.
    r.cache = ByteLru::new(CACHE_BYTES);
    let mut buffer = vec![0; 192 * 108 * 4];
    write(&mut r, 1.0, &mut buffer).unwrap();
    let profiled = r.stats();
    assert!(timings(&profiled)
        .iter()
        .all(|elapsed| *elapsed > Duration::ZERO));
    assert_eq!(counters(&profiled), [2, 0, 2, 0, 2]);

    r.cache = ByteLru::new(CACHE_BYTES);
    r.png(1.0).unwrap();
    r.png(1.0).unwrap();
    assert_eq!(timings(&r.stats()), timings(&profiled));
    assert_eq!(counters(&r.stats()), [2, 1, 3, 1, 3]);

    r.set_profiling(false);
    r.cache = ByteLru::new(CACHE_BYTES);
    write(&mut r, 1.0, &mut buffer).unwrap();
    assert_eq!(timings(&r.stats()), timings(&profiled));
    assert_eq!(counters(&r.stats()), [3, 1, 4, 1, 4]);
    // Returned stats are snapshots, not live handles.
    assert_eq!(counters(&profiled), [2, 0, 2, 0, 2]);
}

fn assert_premultiplied_bgra(straight: &[u8], premultiplied: &[u8]) {
    assert_eq!(straight.len(), premultiplied.len());
    for (rgba, bgra) in straight.chunks_exact(4).zip(premultiplied.chunks_exact(4)) {
        assert_eq!(rgba[3], bgra[3]);
        for (channel, straight_channel) in [2, 1, 0].into_iter().enumerate() {
            let expected = (u16::from(rgba[straight_channel]) * u16::from(rgba[3]) + 127) / 255;
            // Unpremultiplication and re-quantization can differ by one byte.
            assert!(
                bgra[channel].abs_diff(expected as u8) <= 1,
                "{rgba:?} vs {bgra:?}"
            );
            assert!(bgra[channel] <= bgra[3], "{bgra:?}");
        }
    }
}

#[test]
fn premultiplied_bgra_has_explicit_channel_order_for_both_native_surface_formats() {
    let colors = [
        Color::from_argb(255, 231, 83, 17),
        Color::from_argb(128, 210, 122, 34),
        Color::from_argb(1, 255, 0, 0),
        Color::from_argb(0, 19, 91, 203),
    ];
    let mut s = surface(4, 1, CACHE_BYTES).unwrap();
    for (x, color) in colors.into_iter().enumerate() {
        let mut paint = Paint::default();
        paint.set_color(color).set_blend_mode(BlendMode::Src);
        s.canvas()
            .draw_rect(Rect::from_xywh(x as f32, 0.0, 1.0, 1.0), &paint);
    }
    let image = TestImage::from_surface(s);
    let mut event = image.event();
    event.to = event.from;
    event.width = 40.0;
    event.height = 10.0;
    event.opacity = 1.0;
    for color_type in [ColorType::RGBA8888, ColorType::BGRA8888] {
        let mut r = Renderer::new(
            192,
            108,
            RenderInput {
                documents: vec![],
                images: vec![event.clone()],
            },
            vec![],
        )
        .unwrap();
        // Exercise both N32 storage possibilities on every test platform.
        r.surface = surfaces::raster(
            &ImageInfo::new((192, 108), color_type, AlphaType::Premul, None),
            None,
            None,
        )
        .unwrap();
        let mut bgra = vec![0xa5; 192 * 108 * 4];
        r.frame_premultiplied_bgra_into(1.0, &mut bgra).unwrap();
        assert_eq!(&bgra[..4], &[17, 83, 231, 255]);
        for (actual, expected) in bgra[4..8].iter().zip([17u8, 61, 105, 128]) {
            assert!(actual.abs_diff(expected) <= 1);
        }
        assert_eq!(&bgra[8..12], &[0, 0, 1, 1]);
        assert!(bgra[12..].iter().all(|v| *v == 0));
        // Compare directly with stored premultiplied pixels, without a straight
        // intermediate or assumptions about native channel order/row padding.
        let native = r.surface.peek_pixels().unwrap();
        assert_eq!(native.alpha_type(), AlphaType::Premul);
        for (row, packed) in native
            .bytes()
            .unwrap()
            .chunks(native.row_bytes())
            .zip(bgra.chunks(192 * 4))
        {
            for (stored, output) in row[..192 * 4].chunks_exact(4).zip(packed.chunks_exact(4)) {
                let expected = if native.color_type() == ColorType::RGBA8888 {
                    [stored[2], stored[1], stored[0], stored[3]]
                } else {
                    stored.try_into().unwrap()
                };
                assert_eq!(output, expected);
            }
        }
        let rgba = r.frame(1.0).unwrap();
        assert_eq!(&rgba[..4], &[231, 83, 17, 255]);
        assert_premultiplied_bgra(&rgba, &bgra);
    }
}

#[test]
fn premultiplied_bgra_reuses_buffers_for_translucent_effects_images_and_seeks() {
    let image = TestImage::new();
    let style = STYLE
        .replace("&H00FFFFFF", "&H804020E0")
        .replace("&H00000000", "&H8020D040")
        .replace("&H80000000", "&H80E04020")
        .replace(",1,3,2,2,", ",1,6,8,2,");
    let mut source = input("Text 😀", &style);
    source.images.push(image.event());
    let mut r = Renderer::new(960, 540, source.clone(), vec![]).unwrap();
    let mut reference = Renderer::new(960, 540, source, vec![]).unwrap();
    let preview = reference.png(1.0).unwrap();
    let mut buffer = vec![0xa5; 960 * 540 * 4];
    let pointer = buffer.as_ptr();
    for time in [0.0, 1.0, 1.5, 2.99, 3.0, 8.0, 1.5, 0.99, 1.0] {
        r.frame_premultiplied_bgra_into(time, &mut buffer).unwrap();
        assert_eq!(buffer.as_ptr(), pointer);
        let straight = reference.frame(time).unwrap();
        assert_premultiplied_bgra(&straight, &buffer);
        if (1.0..3.0).contains(&time) {
            // Isolate the subtitle region from the moving image. Red fill,
            // green outline and blue shadow must each be present and translucent.
            let text = &buffer[400 * 960 * 4..];
            for dominant in 0..3 {
                assert!(
                    text.chunks_exact(4).any(|p| {
                        p[3] > 30
                            && p[3] < 200
                            && (0..3)
                                .all(|c| c == dominant || p[dominant] > p[c].saturating_add(20))
                    }),
                    "missing translucent channel {dominant} at {time}"
                );
            }
        } else {
            assert!(buffer.iter().all(|byte| *byte == 0));
        }
    }
    assert_eq!(counters(&r.stats()), [9, 4, 1, 4, 1]);
    assert_eq!(r.png(1.0).unwrap(), preview);
    let straight = r.frame(1.0).unwrap();
    assert_eq!(straight, reference.frame(1.0).unwrap());
    let png = Image::from_encoded(Data::new_copy(&preview)).unwrap();
    let info = ImageInfo::new((960, 540), ColorType::RGBA8888, AlphaType::Unpremul, None);
    let mut decoded = vec![0; straight.len()];
    assert!(png.read_pixels(
        &info,
        &mut decoded,
        960 * 4,
        (0, 0),
        skia_safe::image::CachingHint::Disallow
    ));
    assert!(straight
        .iter()
        .zip(decoded)
        .all(|(a, b)| a.abs_diff(b) <= 1));
    let before = r.stats().frames;
    for time in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        buffer.fill(0xa5);
        assert!(r.frame_premultiplied_bgra_into(time, &mut buffer).is_err());
        assert!(buffer.iter().all(|byte| *byte == 0xa5));
    }
    assert_eq!(r.stats().frames, before);
}

#[test]
fn image_trajectory_alpha_and_end_boundary() {
    let path = std::env::temp_dir().join(format!(
        "native-overlay-image-test-{}.png",
        std::process::id()
    ));
    let mut s = surface(8, 8, CACHE_BYTES).unwrap();
    s.canvas().clear(Color::RED);
    fs::write(
        &path,
        s.image_snapshot()
            .encode(None, EncodedImageFormat::PNG, None)
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    let input = RenderInput {
        documents: vec![],
        images: vec![ImageEvent {
            path: path.clone(),
            start: 0.0,
            end: 2.0,
            from: (0.0, 0.0),
            to: (960.0, 540.0),
            width: 192.0,
            height: 192.0,
            opacity: 0.5,
        }],
    };
    let mut r = Renderer::new(192, 108, input, vec![]).unwrap();
    let data = r.frame(1.0).unwrap();
    let b = bbox(&data, 192).unwrap();
    assert_eq!((b.0, b.1), (48, 27));
    assert!(data
        .chunks_exact(4)
        .any(|p| p[0] == 255 && (127..=128).contains(&p[3])));
    assert!(r.frame(2.0).unwrap().iter().all(|v| *v == 0));
    fs::remove_file(path).unwrap();
}
