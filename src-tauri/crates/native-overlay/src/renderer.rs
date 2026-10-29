use crate::{
    ass::{self, Event},
    state::{alignment, ByteLru, Timeline},
    ImageEvent, RenderInput, SourceKind,
};
use skia_safe::{
    color_filters,
    font_style::{Slant, Weight, Width},
    image_filters, surfaces,
    textlayout::{
        FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, TextAlign, TextDecoration,
        TextStyle, TypefaceFontProvider,
    },
    AlphaType, BlendMode, Color, ColorType, Data, EncodedImageFormat, FontMgr, FontStyle, Image,
    ImageInfo, Paint, Rect, Surface,
};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation;

const CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_FRAME_BYTES: usize = 128 * 1024 * 1024;
// Allow five explicit text surfaces plus effect-filter intermediates within a
// separate 64 MiB working budget, in addition to the retained sprite LRU.
const MAX_TEXT_RASTER_BYTES: usize = CACHE_BYTES / 8;
const EMOJI_FAMILY: &str = "NativeOverlayColorEmoji";

#[derive(Clone, Copy)]
enum FrameFormat {
    StraightRgba,
    PremultipliedBgra,
}

impl FrameFormat {
    fn readback(self) -> (ColorType, AlphaType, &'static str) {
        match self {
            Self::StraightRgba => (ColorType::RGBA8888, AlphaType::Unpremul, "straight RGBA"),
            Self::PremultipliedBgra => {
                (ColorType::BGRA8888, AlphaType::Premul, "premultiplied BGRA")
            }
        }
    }
}

#[derive(Clone)]
struct Sprite {
    image: Image,
    /// Content box dimensions, excluding raster padding, in output pixels.
    size: (f32, f32),
    pad: f32,
}

struct Fonts {
    collection: FontCollection,
    emoji: HashSet<String>,
    supplied: Vec<String>,
}

impl Fonts {
    fn new(paths: Vec<PathBuf>) -> Result<Self, String> {
        if paths.len() > 128 {
            return Err("At most 128 font files may be provided".into());
        }
        let manager = FontMgr::new();
        let mut provider = TypefaceFontProvider::new();
        let face = manager
            .new_from_data(include_bytes!("../assets/Noto-COLRv1.ttf"), None)
            .ok_or("Cannot load bundled Noto COLRv1 font")?;
        provider.register_typeface(face, EMOJI_FAMILY);
        let mut supplied = Vec::new();
        let mut total = 0;
        for path in paths {
            let bytes = read_bounded(&path, CACHE_BYTES)?;
            total += bytes.len();
            if total > 256 * 1024 * 1024 {
                return Err("Provided fonts exceed 256 MiB".into());
            }
            // OpenType collections have a big-endian face count at byte 8.
            let faces = if bytes.starts_with(b"ttcf") {
                bytes
                    .get(8..12)
                    .map(|b| u32::from_be_bytes(b.try_into().unwrap()) as usize)
                    .unwrap_or(0)
            } else {
                1
            };
            if faces == 0 || faces > 128 {
                return Err(format!("Invalid font collection: {}", path.display()));
            }
            for index in 0..faces {
                let face = manager
                    .new_from_data(&bytes, index)
                    .ok_or_else(|| format!("Cannot load font {} (face {index})", path.display()))?;
                let family = face.family_name();
                // Preserve localized family names, e.g. 微软雅黑 requested by danmu.
                for localized in face.new_family_name_iterator() {
                    provider.register_typeface(face.clone(), localized.string.as_str());
                }
                provider.register_typeface(face, None);
                if !supplied.contains(&family) {
                    supplied.push(family);
                }
            }
        }
        let mut collection = FontCollection::new();
        collection.set_asset_font_manager(Some(provider.into()));
        collection.set_default_font_manager(Some(manager), None);
        // The sprite LRU owns retention. Do not also retain all historical
        // shaped paragraphs inside SkParagraph's separate cache.
        collection.paragraph_cache_mut().turn_on(false);
        let emoji = include_str!("../assets/emoji-test.txt")
            .lines()
            .filter_map(|line| {
                let (codes, _) = line.split('#').next()?.split_once(';')?;
                codes
                    .split_whitespace()
                    .map(|code| char::from_u32(u32::from_str_radix(code, 16).ok()?))
                    .collect::<Option<String>>()
            })
            .collect();
        Ok(Self {
            collection,
            emoji,
            supplied,
        })
    }

    fn paragraph(&self, event: &Event, mask: bool) -> Paragraph {
        let mut style = ParagraphStyle::new();
        style.set_text_align(match (event.style.alignment - 1) % 3 {
            0 => TextAlign::Left,
            1 => TextAlign::Center,
            _ => TextAlign::Right,
        });
        let mut collection = self.collection.clone();
        let styles: Vec<_> = event
            .runs
            .iter()
            .map(|run| {
                let mut families = vec![run.font.as_str()];
                families.extend(self.supplied.iter().map(String::as_str));
                families.extend([
                    "sans-serif",
                    "Noto Sans CJK SC",
                    "Microsoft YaHei",
                    "PingFang SC",
                ]);
                // ASS Fontsize is a cell height, not an OpenType em. Use the
                // selected body face's ascent+descent, also for inline fonts.
                let em_ratio = collection
                    .find_typefaces(&families, FontStyle::normal())
                    .first()
                    .map(|face| {
                        let (_, metrics) =
                            skia_safe::Font::from_typeface(face.clone(), 1.0).metrics();
                        let cell = metrics.descent - metrics.ascent;
                        if cell.is_finite() && cell > 0.0 {
                            1.0 / cell
                        } else {
                            1.0
                        }
                    })
                    .unwrap_or(1.0);
                let mut body = TextStyle::new();
                body.set_font_families(&families)
                    .set_font_size(run.size * em_ratio)
                    .set_letter_spacing(event.style.spacing)
                    .set_color(Color::new(if mask {
                        run.color | 0xff000000
                    } else {
                        run.color
                    }))
                    .set_font_style(FontStyle::new(
                        if run.bold {
                            Weight::BOLD
                        } else {
                            Weight::NORMAL
                        },
                        Width::NORMAL,
                        if run.italic {
                            Slant::Italic
                        } else {
                            Slant::Upright
                        },
                    ));
                let mut decoration = TextDecoration::NO_DECORATION;
                if run.underline {
                    decoration |= TextDecoration::UNDERLINE;
                }
                if run.strike {
                    decoration |= TextDecoration::LINE_THROUGH;
                }
                body.set_decoration_type(decoration);
                let mut emoji = body.clone();
                emoji
                    .set_font_families(&[EMOJI_FAMILY])
                    .set_font_style(FontStyle::normal());
                (body, emoji)
            })
            .collect();
        let mut builder = ParagraphBuilder::new(&style, collection);
        // Segment the entire visible text, not each ASS run: an inline tag
        // inside a ZWJ/skin-tone sequence must not split its grapheme. The
        // grapheme inherits the style at its first scalar.
        let text: String = event.runs.iter().map(|r| r.text.as_str()).collect();
        let mut ends = Vec::with_capacity(event.runs.len());
        let mut end = 0;
        for run in &event.runs {
            end += run.text.len();
            ends.push(end);
        }
        let mut index = 0;
        let mut buffered = String::new();
        let mut previous = (0, false);
        for (offset, grapheme) in text.grapheme_indices(true) {
            while ends[index] <= offset {
                index += 1;
            }
            let current = (index, self.emoji.contains(grapheme));
            if !buffered.is_empty() && current != previous {
                let (body, emoji) = &styles[previous.0];
                builder
                    .push_style(if previous.1 { emoji } else { body })
                    .add_text(&buffered)
                    .pop();
                buffered.clear();
            }
            buffered.push_str(grapheme);
            previous = current;
        }
        if !buffered.is_empty() {
            let (body, emoji) = &styles[previous.0];
            builder
                .push_style(if previous.1 { emoji } else { body })
                .add_text(&buffered)
                .pop();
        }
        builder.build()
    }
}

/// Cumulative renderer work. Durations are collected only while profiling is
/// enabled, and exclude PNG calls. Cache counters include PNG cache accesses.
#[derive(Clone, Debug, Default)]
pub struct RenderStats {
    pub frames: u64,
    pub text_layout: Duration,
    pub text_raster: Duration,
    pub image_prepare: Duration,
    pub composition: Duration,
    pub readback: Duration,
    pub text_cache_hits: u64,
    pub text_cache_misses: u64,
    pub image_cache_hits: u64,
    pub image_cache_misses: u64,
}

/// CPU-only Skia compositor. Owns fonts, one output surface and a 64 MiB
/// byte-accounted LRU, not a raster for every event in a movie.
pub struct Renderer {
    width: u32,
    height: u32,
    surface: Surface,
    fonts: Fonts,
    events: Vec<Event>,
    images: Vec<ImageEvent>,
    timeline: Timeline,
    image_timeline: Timeline,
    cache: ByteLru<Sprite>,
    image_sprite_ids: Vec<usize>,
    active_text: Vec<usize>,
    active_images: Vec<usize>,
    placed: Vec<(i32, u8, Rect)>,
    profiling: bool,
    stats: RenderStats,
    missing_glyphs: usize,
}

impl Renderer {
    pub fn new(
        width: u32,
        height: u32,
        input: RenderInput,
        font_paths: Vec<PathBuf>,
    ) -> Result<Self, String> {
        raster_bytes(width as f32, height as f32, MAX_FRAME_BYTES)?;
        let mut events = Vec::new();
        for source in input.documents {
            if source.text.len() > 128 * 1024 * 1024 {
                return Err("ASS document exceeds 128 MiB".into());
            }
            let offset = events.len();
            let mut parsed = ass::parse(&source.text, source.kind)?;
            for event in &mut parsed {
                if event.runs.iter().map(|run| run.text.len()).sum::<usize>() > 1024 * 1024 {
                    return Err("ASS dialogue text exceeds 1 MiB".into());
                }
                event.order += offset;
            }
            events.extend(parsed);
        }
        events.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.order.cmp(&b.order)));
        let mut images = input.images;
        for image in &images {
            validate_image(image)?;
        }
        images.sort_by(|a, b| a.start.total_cmp(&b.start));
        // Paths name immutable preprocessed assets for this renderer. Intern
        // once, using the actual (ceil-rounded) output raster dimensions, not
        // scheduling, placement, opacity or per-frame filesystem inspection.
        let mut sprite_ids = HashMap::new();
        let mut image_sprite_ids = Vec::with_capacity(images.len());
        for image in &images {
            let (w, h, _) = raster_bytes(
                image.width * width as f32 / 1920.0,
                image.height * height as f32 / 1080.0,
                CACHE_BYTES,
            )?;
            let next = events.len() + sprite_ids.len();
            let id = *sprite_ids.entry((image.path.clone(), w, h)).or_insert(next);
            image_sprite_ids.push(id);
        }
        let timeline = Timeline::new(events.iter().map(|e| (e.start, e.end)).collect());
        let image_timeline = Timeline::new(images.iter().map(|e| (e.start, e.end)).collect());
        Ok(Self {
            width,
            height,
            surface: surface(width as i32, height as i32, MAX_FRAME_BYTES)?,
            fonts: Fonts::new(font_paths)?,
            events,
            images,
            timeline,
            image_timeline,
            cache: ByteLru::new(CACHE_BYTES),
            image_sprite_ids,
            active_text: Vec::new(),
            active_images: Vec::new(),
            placed: Vec::new(),
            profiling: false,
            stats: RenderStats::default(),
            missing_glyphs: 0,
        })
    }

    /// Count of unresolved glyphs encountered on cache misses. Skia paints
    /// visible .notdef glyphs; a missing font/emoji never aborts an export.
    pub fn missing_glyph_count(&self) -> usize {
        self.missing_glyphs
    }

    /// Enable stage timings for subsequent raw frames, without resetting stats.
    /// Disabled by default; PNG rendering never contributes timings or frames.
    pub fn set_profiling(&mut self, enabled: bool) {
        self.profiling = enabled;
    }

    /// Snapshot cumulative work; completed RGBA/BGRA frames and cache accesses count
    /// even when timings are disabled. No per-frame logging is performed.
    pub fn stats(&self) -> RenderStats {
        self.stats.clone()
    }

    /// Straight (unpremultiplied) RGBA8888, tightly packed row-major.
    pub fn frame(&mut self, time: f64) -> Result<Vec<u8>, String> {
        let mut bytes = vec![0; self.width as usize * self.height as usize * 4];
        self.frame_into(time, &mut bytes)?;
        Ok(bytes)
    }

    /// Render into exactly width × height × 4 bytes of straight RGBA8888.
    /// Invalid lengths are rejected before composition or timeline/cache changes.
    pub fn frame_into(&mut self, time: f64, bytes: &mut [u8]) -> Result<(), String> {
        self.frame_into_format(time, bytes, FrameFormat::StraightRgba)
    }

    /// Render into exactly width × height × 4 premultiplied BGRA8888 bytes.
    /// Pixels are B, G, R, A bytes on every platform, tightly packed row-major.
    /// Reads Skia's premultiplied surface directly, without unpremultiplication.
    /// Invalid lengths are rejected before composition or timeline/cache changes.
    pub fn frame_premultiplied_bgra_into(
        &mut self,
        time: f64,
        bytes: &mut [u8],
    ) -> Result<(), String> {
        self.frame_into_format(time, bytes, FrameFormat::PremultipliedBgra)
    }

    fn frame_into_format(
        &mut self,
        time: f64,
        bytes: &mut [u8],
        format: FrameFormat,
    ) -> Result<(), String> {
        let (color_type, alpha_type, name) = format.readback();
        let expected = self.width as usize * self.height as usize * 4;
        if bytes.len() != expected {
            return Err(format!(
                "{name} buffer length {} does not match expected {expected}",
                bytes.len()
            ));
        }
        let profile = self.profiling;
        let started = profile.then(Instant::now);
        let preparation =
            self.stats.text_layout + self.stats.text_raster + self.stats.image_prepare;
        let result = self.compose(time, profile);
        if let Some(started) = started {
            let nested = self.stats.text_layout + self.stats.text_raster + self.stats.image_prepare
                - preparation;
            self.stats.composition += started.elapsed().saturating_sub(nested);
        }
        result?;
        let started = profile.then(Instant::now);
        let info = ImageInfo::new(
            (self.width as i32, self.height as i32),
            color_type,
            alpha_type,
            None,
        );
        let read = self
            .surface
            .read_pixels(&info, bytes, self.width as usize * 4, (0, 0));
        if let Some(started) = started {
            self.stats.readback += started.elapsed();
        }
        if !read {
            return Err(format!("Skia {name} readback failed"));
        }
        self.stats.frames += 1;
        Ok(())
    }

    /// PNG from exactly the same compositor used by `frame`.
    pub fn png(&mut self, time: f64) -> Result<Vec<u8>, String> {
        self.compose(time, false)?;
        self.surface
            .image_snapshot()
            .encode(None, EncodedImageFormat::PNG, None)
            .map(|data| data.as_bytes().to_vec())
            .ok_or_else(|| "PNG encoding failed".into())
    }

    fn text_sprite(&mut self, id: usize, profile: bool) -> Result<Sprite, String> {
        if let Some(sprite) = self.cache.get(id) {
            self.stats.text_cache_hits += 1;
            return Ok(sprite);
        }
        self.stats.text_cache_misses += 1;
        let event = &self.events[id];
        let style = &event.style;
        let sx = self.width as f32 / event.res.0 * style.scale_x;
        let sy = self.height as f32 / event.res.1 * style.scale_y;
        let wrap = if event.movement.is_some() {
            16384.0
        } else {
            (event.res.0 - style.margins[0] - style.margins[1]) / style.scale_x
        };
        if !wrap.is_finite() || wrap <= 0.0 {
            return Err("Subtitle margins leave no layout width".into());
        }
        let started = profile.then(Instant::now);
        let mut paragraph = self.fonts.paragraph(event, false);
        paragraph.layout(wrap);
        let layout_width = paragraph.longest_line().ceil().max(1.0) + 1.0;
        paragraph.layout(layout_width);
        if let Some(started) = started {
            self.stats.text_layout += started.elapsed();
        }
        let missing = paragraph.unresolved_glyphs().unwrap_or(0);
        if missing > 0 {
            self.missing_glyphs = self.missing_glyphs.saturating_add(missing);
            log::warn!("native-overlay: {missing} unresolved glyphs in event {} (visible missing-glyph fallback)", event.order);
        }
        let size = (layout_width * sx, paragraph.height().max(1.0) * sy);
        let outline = style.outline * self.height as f32 / event.res.1;
        let shadow = style.shadow * self.height as f32 / event.res.1;
        let largest_size = event
            .runs
            .iter()
            .map(|run| run.size)
            .fold(style.size, f32::max);
        let pad = (largest_size * sx.max(sy) * 0.5 + outline + shadow + 3.0).ceil();
        let (w, h, bytes) = raster_bytes(
            size.0 + pad * 2.0,
            size.1 + pad * 2.0,
            MAX_TEXT_RASTER_BYTES,
        )?;
        let started = profile.then(Instant::now);
        let mut fill = surface(w, h, CACHE_BYTES)?;
        fill.canvas().translate((pad, pad)).scale((sx, sy));
        paragraph.paint(fill.canvas(), (0.0, 0.0));
        let fill = fill.image_snapshot();
        if let Some(started) = started {
            self.stats.text_raster += started.elapsed();
        }
        let started = profile.then(Instant::now);
        let mut mask = self.fonts.paragraph(event, true);
        mask.layout(layout_width);
        if let Some(started) = started {
            self.stats.text_layout += started.elapsed();
        }
        let started = profile.then(Instant::now);
        let mut alpha = surface(w, h, CACHE_BYTES)?;
        alpha.canvas().translate((pad, pad)).scale((sx, sy));
        mask.paint(alpha.canvas(), (0.0, 0.0));
        let alpha = alpha.image_snapshot();
        let mut styled = surface(w, h, CACHE_BYTES)?;
        let canvas = styled.canvas();
        if style.border == 3 {
            let rect = Rect::from_xywh(
                pad - outline,
                pad - outline,
                size.0 + 2.0 * outline,
                size.1 + 2.0 * outline,
            );
            let mut paint = Paint::default();
            if shadow > 0.0 {
                paint.set_color(Color::new(style.back));
                canvas.save();
                canvas.translate((shadow, shadow));
                canvas.draw_rect(rect, &paint);
                canvas.restore();
            }
            paint.set_color(Color::new(style.outline_color));
            canvas.draw_rect(rect, &paint);
        } else {
            let mut dilation = Paint::default();
            if outline > 0.0 {
                dilation.set_image_filter(image_filters::dilate((outline, outline), None, None));
            }
            let mut silhouette = surface(w, h, CACHE_BYTES)?;
            silhouette
                .canvas()
                .draw_image(&alpha, (0.0, 0.0), Some(&dilation));
            let silhouette = silhouette.image_snapshot();
            let mut erase = Paint::default();
            erase.set_blend_mode(BlendMode::DstOut);
            // Knock out the foreground before tinting effects. Otherwise an
            // 80%-opaque fill over an 80%-opaque outline becomes 96% opaque.
            if shadow > 0.0 {
                let mut effect = surface(w, h, CACHE_BYTES)?;
                effect
                    .canvas()
                    .draw_image(&silhouette, (shadow, shadow), None);
                effect
                    .canvas()
                    .draw_image(&silhouette, (0.0, 0.0), Some(&erase));
                canvas.draw_image(
                    effect.image_snapshot(),
                    (0.0, 0.0),
                    Some(&mask_paint(style.back)),
                );
            }
            if outline > 0.0 {
                let mut effect = surface(w, h, CACHE_BYTES)?;
                effect.canvas().draw_image(&silhouette, (0.0, 0.0), None);
                effect.canvas().draw_image(&alpha, (0.0, 0.0), Some(&erase));
                canvas.draw_image(
                    effect.image_snapshot(),
                    (0.0, 0.0),
                    Some(&mask_paint(style.outline_color)),
                );
            }
        }
        canvas.draw_image(&fill, (0.0, 0.0), None);
        let sprite = Sprite {
            image: styled.image_snapshot(),
            size,
            pad,
        };
        if let Some(started) = started {
            self.stats.text_raster += started.elapsed();
        }
        self.cache.insert(id, sprite.clone(), bytes);
        Ok(sprite)
    }

    fn image_sprite(&mut self, id: usize, profile: bool) -> Result<Sprite, String> {
        let key = self.image_sprite_ids[id];
        if let Some(sprite) = self.cache.get(key) {
            self.stats.image_cache_hits += 1;
            return Ok(sprite);
        }
        self.stats.image_cache_misses += 1;
        let started = profile.then(Instant::now);
        let event = &self.images[id];
        let bytes = read_bounded(&event.path, 32 * 1024 * 1024)?;
        let image = Image::from_encoded(Data::new_copy(&bytes))
            .ok_or_else(|| format!("Cannot decode {}", event.path.display()))?;
        raster_bytes(image.width() as f32, image.height() as f32, CACHE_BYTES)?;
        let image = image
            .make_raster_image(None, None)
            .ok_or_else(|| format!("Cannot decode pixels of {}", event.path.display()))?;
        let size = (
            event.width * self.width as f32 / 1920.0,
            event.height * self.height as f32 / 1080.0,
        );
        let (w, h, bytes) = raster_bytes(size.0, size.1, CACHE_BYTES)?;
        let mut output = surface(w, h, CACHE_BYTES)?;
        // Keep the scheduler's footprint, but preserve the decoded artwork's
        // aspect ratio inside it (the previous FFmpeg chain used decrease+pad).
        let scale = (w as f32 / image.width() as f32).min(h as f32 / image.height() as f32);
        let content = (image.width() as f32 * scale, image.height() as f32 * scale);
        output.canvas().draw_image_rect(
            &image,
            None,
            Rect::from_xywh(
                (w as f32 - content.0) / 2.0,
                (h as f32 - content.1) / 2.0,
                content.0,
                content.1,
            ),
            &Paint::default(),
        );
        let sprite = Sprite {
            image: output.image_snapshot(),
            size,
            pad: 0.0,
        };
        if let Some(started) = started {
            self.stats.image_prepare += started.elapsed();
        }
        self.cache.insert(key, sprite.clone(), bytes);
        Ok(sprite)
    }

    fn compose(&mut self, time: f64, profile: bool) -> Result<(), String> {
        if !time.is_finite() {
            return Err("Frame time must be finite".into());
        }
        self.surface.canvas().clear(Color::TRANSPARENT);
        self.active_text.clear();
        self.active_text.extend_from_slice(self.timeline.at(time));
        self.active_text.sort_by(|a, b| {
            self.events[*a]
                .layer
                .cmp(&self.events[*b].layer)
                .then(self.events[*a].order.cmp(&self.events[*b].order))
        });
        self.placed.clear();
        for index in 0..self.active_text.len() {
            let id = self.active_text[index];
            if self.events[id].runs.is_empty() {
                continue;
            }
            let sprite = self.text_sprite(id, profile)?;
            let event = &self.events[id];
            let (ax, ay) = alignment(event.style.alignment);
            let (xscale, yscale) = (
                self.width as f32 / event.res.0,
                self.height as f32 / event.res.1,
            );
            let anchor = if let Some([from, to]) = event.movement {
                let f = ((time - event.start) / (event.end - event.start)).clamp(0.0, 1.0) as f32;
                (
                    (from.0 + (to.0 - from.0) * f) * xscale,
                    (from.1 + (to.1 - from.1) * f) * yscale,
                )
            } else {
                let m = event.style.margins;
                (
                    (m[0] + (event.res.0 - m[0] - m[1]) * ax) * xscale,
                    if ay == 0.0 {
                        m[2] * yscale
                    } else if ay == 1.0 {
                        self.height as f32 - m[2] * yscale
                    } else {
                        self.height as f32 / 2.0
                    },
                )
            };
            let outline = event.style.outline * yscale;
            let shadow = event.style.shadow * yscale;
            let mut rect = rotated_bounds(
                Rect::from_xywh(
                    -sprite.size.0 * ax - outline,
                    -sprite.size.1 * ay - outline,
                    sprite.size.0 + 2.0 * outline + shadow,
                    sprite.size.1 + 2.0 * outline + shadow,
                ),
                -event.style.angle,
            );
            rect.offset(anchor);
            let mut dy = 0.0;
            if event.kind == SourceKind::Subtitle && event.movement.is_none() {
                dy = stack(&mut rect, event.layer, event.style.alignment, &self.placed);
                self.placed.push((event.layer, event.style.alignment, rect));
            }
            let canvas = self.surface.canvas();
            canvas.save();
            canvas.translate((anchor.0, anchor.1 + dy));
            // ASS positive rotation is counterclockwise in screen coordinates.
            canvas.rotate(-event.style.angle, None);
            canvas.draw_image(
                &sprite.image,
                (
                    -sprite.size.0 * ax - sprite.pad,
                    -sprite.size.1 * ay - sprite.pad,
                ),
                None,
            );
            canvas.restore();
        }
        self.active_images.clear();
        self.active_images
            .extend_from_slice(self.image_timeline.at(time));
        for index in 0..self.active_images.len() {
            let id = self.active_images[index];
            let sprite = self.image_sprite(id, profile)?;
            let event = &self.images[id];
            let f = ((time - event.start) / (event.end - event.start)).clamp(0.0, 1.0) as f32;
            let x = (event.from.0 + (event.to.0 - event.from.0) * f) * self.width as f32 / 1920.0;
            let y = (event.from.1 + (event.to.1 - event.from.1) * f) * self.height as f32 / 1080.0;
            let mut paint = Paint::default();
            paint.set_alpha_f(event.opacity);
            self.surface
                .canvas()
                .draw_image(&sprite.image, (x, y), Some(&paint));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

fn mask_paint(color: u32) -> Paint {
    let mut paint = Paint::default();
    paint.set_color_filter(color_filters::blend(Color::new(color), BlendMode::SrcIn));
    paint
}

fn rotated_bounds(rect: Rect, degrees: f32) -> Rect {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let mut result = Rect::new(
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    );
    for (x, y) in [
        (rect.left, rect.top),
        (rect.right, rect.top),
        (rect.left, rect.bottom),
        (rect.right, rect.bottom),
    ] {
        let (x, y) = (x * cos - y * sin, x * sin + y * cos);
        result.left = result.left.min(x);
        result.right = result.right.max(x);
        result.top = result.top.min(y);
        result.bottom = result.bottom.max(y);
    }
    result
}

fn stack(rect: &mut Rect, layer: i32, alignment: u8, placed: &[(i32, u8, Rect)]) -> f32 {
    let original = rect.top;
    let upwards = alignment <= 3;
    // Each shift passes at least one previous box; at most N shifts are needed.
    for _ in 0..placed.len() {
        let collision = placed
            .iter()
            .find(|(l, a, r)| *l == layer && *a == alignment && rect.intersects(*r));
        let Some((_, _, other)) = collision else {
            break;
        };
        let dy = if upwards {
            other.top - rect.bottom - 2.0
        } else {
            other.bottom - rect.top + 2.0
        };
        rect.offset((0.0, dy));
    }
    rect.top - original
}

fn validate_image(e: &ImageEvent) -> Result<(), String> {
    if !e.start.is_finite()
        || !e.end.is_finite()
        || e.start < 0.0
        || e.end <= e.start
        || ![
            e.from.0, e.from.1, e.to.0, e.to.1, e.width, e.height, e.opacity,
        ]
        .iter()
        .all(|x| x.is_finite())
        || e.width <= 0.0
        || e.height <= 0.0
        || !(0.0..=1.0).contains(&e.opacity)
    {
        return Err("Invalid image event: expected finite coordinates, positive dimensions and half-open time range, opacity 0..1".into());
    }
    Ok(())
}

fn raster_bytes(width: f32, height: f32, limit: usize) -> Result<(i32, i32, usize), String> {
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
        || width > 16384.0
        || height > 16384.0
    {
        return Err(format!("Unsafe raster dimensions {width}x{height}"));
    }
    let (w, h) = (width.ceil() as i32, height.ceil() as i32);
    let bytes = (w as usize)
        .checked_mul(h as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or("Raster size overflow")?;
    if bytes > limit {
        return Err(format!(
            "Raster {w}x{h} exceeds {limit}-byte allocation limit"
        ));
    }
    Ok((w, h, bytes))
}

fn surface(w: i32, h: i32, limit: usize) -> Result<Surface, String> {
    raster_bytes(w as f32, h as f32, limit)?;
    // N32 is native RGBA or BGRA, always premultiplied here. Preserve this
    // storage for existing drawing/PNG behavior; readback explicitly selects
    // byte order. BGRA/Premul needs only a copy or swizzle, not unpremultiplication.
    let mut surface =
        surfaces::raster_n32_premul((w, h)).ok_or("Cannot allocate CPU Skia raster")?;
    surface.canvas().clear(Color::TRANSPARENT);
    Ok(surface)
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if file.metadata().map_err(|e| e.to_string())?.len() > limit as u64 {
        return Err(format!(
            "{} exceeds {limit}-byte file limit",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("File grew past allocation limit while reading".into());
    }
    Ok(bytes)
}
