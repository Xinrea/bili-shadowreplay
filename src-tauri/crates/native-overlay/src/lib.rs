mod ass;
mod renderer;
mod state;

use std::path::PathBuf;

/// The provenance of an ASS document, used for default layout policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceKind {
    Subtitle,
    Danmaku,
}

/// An application-generated ASS document and its rendering class.
#[derive(Clone, Debug)]
pub struct AssSource {
    pub text: String,
    pub kind: SourceKind,
}

/// An independently moving image event. Coordinates and dimensions use
/// normalized 1920 × 1080 canvas pixels.
#[derive(Clone, Debug)]
pub struct ImageEvent {
    pub path: PathBuf,
    pub start: f64,
    pub end: f64,
    pub from: (f32, f32),
    pub to: (f32, f32),
    pub width: f32,
    pub height: f32,
    pub opacity: f32,
}

/// The scene documents and animated image events to composite.
#[derive(Clone, Debug, Default)]
pub struct RenderInput {
    pub documents: Vec<AssSource>,
    pub images: Vec<ImageEvent>,
}

/// Include in persistent frame/segment cache keys. Bump when layout, rendering
/// rules, Skia or bundled assets change.
pub const RENDERER_VERSION: &str = "skia082-noto-e20cbc2-unicode16-v2";

pub use renderer::Renderer;
