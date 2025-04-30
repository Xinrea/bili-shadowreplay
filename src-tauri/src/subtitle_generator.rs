use async_trait::async_trait;
use std::path::Path;

use crate::progress_reporter::ProgressReporterTrait;

pub mod whisper;

// subtitle_generator types
#[allow(dead_code)]
pub enum SubtitleGeneratorType {
    Whisper,
}

impl SubtitleGeneratorType {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            SubtitleGeneratorType::Whisper => "whisper",
        }
    }
    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "whisper" => Some(SubtitleGeneratorType::Whisper),
            _ => None,
        }
    }
}

#[async_trait]
pub trait SubtitleGenerator {
    async fn generate_subtitle(
        &self,
        reporter: &impl ProgressReporterTrait,
        video_path: &Path,
        output_path: &Path,
    ) -> Result<String, String>;
}
