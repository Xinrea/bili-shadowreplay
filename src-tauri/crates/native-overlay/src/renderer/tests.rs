use super::*;
use crate::AssSource;

const FIELDS: &str = "Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding";
const STYLE: &str = "Default,Arial,64,&H00FFFFFF,&H00FFFFFF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,3,2,2,100,100,60,1";

fn input(text: &str, style: &str) -> RenderInput {
    RenderInput { documents: vec![AssSource { kind: SourceKind::Subtitle, text: format!("[Script Info]\nScriptType: v4.00+\nPlayResX: 1920\nPlayResY: 1080\n[V4+ Styles]\nFormat: {FIELDS}\nStyle: {style}\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,{text}\n") }], images: vec![] }
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
    let sprite = r.image_sprite(0).unwrap();
    assert_eq!(sprite.size, (32.0, 16.0));
    assert_eq!((sprite.image.width(), sprite.image.height()), (32, 16));
    assert_eq!(bbox(&r.frame(0.0).unwrap(), 192), Some((0, 4, 31, 11)));
    fs::remove_file(path).unwrap();
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
