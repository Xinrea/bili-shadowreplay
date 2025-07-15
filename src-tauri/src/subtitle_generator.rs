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
    pub subtitle_content: Vec<srtparse::Item>,
}

impl GenerateResult {
    pub fn concat(&mut self, other: &GenerateResult, offset_seconds: u64) {
        let mut to_extend = other.subtitle_content.clone();
        let last_item_index = self.subtitle_content.len();
        for (i, item) in to_extend.iter_mut().enumerate() {
            item.pos = last_item_index + i + 1;
            item.start_time = add_offset(&item.start_time, offset_seconds);
            item.end_time = add_offset(&item.end_time, offset_seconds);
        }
        self.subtitle_content.extend(to_extend);
    }
}

fn add_offset(item: &srtparse::Time, offset: u64) -> srtparse::Time {
    let mut total_seconds = item.seconds + offset;
    let mut total_minutes = item.minutes;
    let mut total_hours = item.hours;

    // Handle seconds overflow (>= 60)
    if total_seconds >= 60 {
        let additional_minutes = total_seconds / 60;
        total_seconds %= 60;
        total_minutes += additional_minutes;
    }

    // Handle minutes overflow (>= 60)
    if total_minutes >= 60 {
        let additional_hours = total_minutes / 60;
        total_minutes %= 60;
        total_hours += additional_hours;
    }

    srtparse::Time {
        hours: total_hours,
        minutes: total_minutes,
        seconds: total_seconds,
        milliseconds: item.milliseconds,
    }
}

pub fn item_to_srt(item: &srtparse::Item) -> String {
    let start_time = format!(
        "{:02}:{:02}:{:02},{:03}",
        item.start_time.hours,
        item.start_time.minutes,
        item.start_time.seconds,
        item.start_time.milliseconds
    );

    let end_time = format!(
        "{:02}:{:02}:{:02},{:03}",
        item.end_time.hours,
        item.end_time.minutes,
        item.end_time.seconds,
        item.end_time.milliseconds
    );

    format!(
        "{}\n{} --> {}\n{}\n\n",
        item.pos, start_time, end_time, item.text
    )
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
        audio_path: &Path,
        language_hint: &str,
    ) -> Result<GenerateResult, String>;
}
