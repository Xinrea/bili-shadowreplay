use async_trait::async_trait;
use std::path::Path;

use crate::progress_reporter::ProgressReporterTrait;

pub mod whisper_cpp;
pub mod whisper_online;

// subtitle_generator types
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum SubtitleGeneratorType {
    Whisper,
    WhisperOnline,
}

#[derive(Debug, Clone)]
pub struct GenerateResult {
    pub generator_type: SubtitleGeneratorType,
    pub subtitle_id: String,
    pub subtitle_content: String,
}

impl SubtitleGeneratorType {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            SubtitleGeneratorType::Whisper => "whisper",
            SubtitleGeneratorType::WhisperOnline => "whisper_online",
        }
    }
    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "whisper" => Some(SubtitleGeneratorType::Whisper),
            "whisper_online" => Some(SubtitleGeneratorType::WhisperOnline),
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
    ) -> Result<GenerateResult, String>;
}
