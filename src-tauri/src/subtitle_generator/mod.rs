use async_trait::async_trait;
use std::path::Path;

use crate::progress::progress_reporter::ProgressReporterTrait;

pub mod powerlive;
pub mod whisper_cpp;
pub mod whisper_online;

// subtitle_generator types
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum SubtitleGeneratorType {
    Whisper,
    WhisperOnline,
    PowerLive,
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
            SubtitleGeneratorType::PowerLive => "powerlive",
        }
    }
    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "whisper" => Some(SubtitleGeneratorType::Whisper),
            "whisper_online" => Some(SubtitleGeneratorType::WhisperOnline),
            "powerlive" => Some(SubtitleGeneratorType::PowerLive),
            _ => None,
        }
    }
}

#[async_trait]
pub trait SubtitleGenerator {
    async fn generate_subtitle(
        &self,
        reporter: Option<&(impl ProgressReporterTrait + 'static)>,
        audio_path: &Path,
        language_hint: &str,
    ) -> Result<GenerateResult, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subtitle_generator_type_as_str() {
        assert_eq!(SubtitleGeneratorType::Whisper.as_str(), "whisper");
        assert_eq!(
            SubtitleGeneratorType::WhisperOnline.as_str(),
            "whisper_online"
        );
        assert_eq!(SubtitleGeneratorType::PowerLive.as_str(), "powerlive");
    }

    #[test]
    fn test_subtitle_generator_type_from_str() {
        assert_eq!(
            SubtitleGeneratorType::from_str("whisper"),
            Some(SubtitleGeneratorType::Whisper)
        );
        assert_eq!(
            SubtitleGeneratorType::from_str("whisper_online"),
            Some(SubtitleGeneratorType::WhisperOnline)
        );
        assert_eq!(
            SubtitleGeneratorType::from_str("powerlive"),
            Some(SubtitleGeneratorType::PowerLive)
        );
        assert_eq!(SubtitleGeneratorType::from_str("unknown"), None);
        assert_eq!(SubtitleGeneratorType::from_str(""), None);
    }

    #[test]
    fn test_subtitle_generator_type_roundtrip() {
        for t in [
            SubtitleGeneratorType::Whisper,
            SubtitleGeneratorType::WhisperOnline,
            SubtitleGeneratorType::PowerLive,
        ] {
            assert_eq!(SubtitleGeneratorType::from_str(t.as_str()), Some(t));
        }
    }

    #[test]
    fn test_add_offset_no_overflow() {
        let time = srtparse::Time {
            hours: 0,
            minutes: 1,
            seconds: 30,
            milliseconds: 500,
        };
        let result = add_offset(&time, 10);
        assert_eq!(result.hours, 0);
        assert_eq!(result.minutes, 1);
        assert_eq!(result.seconds, 40);
        assert_eq!(result.milliseconds, 500);
    }

    #[test]
    fn test_add_offset_seconds_overflow() {
        let time = srtparse::Time {
            hours: 0,
            minutes: 0,
            seconds: 50,
            milliseconds: 0,
        };
        let result = add_offset(&time, 15);
        assert_eq!(result.hours, 0);
        assert_eq!(result.minutes, 1);
        assert_eq!(result.seconds, 5);
        assert_eq!(result.milliseconds, 0);
    }

    #[test]
    fn test_add_offset_minutes_overflow() {
        let time = srtparse::Time {
            hours: 0,
            minutes: 59,
            seconds: 50,
            milliseconds: 0,
        };
        let result = add_offset(&time, 15);
        assert_eq!(result.hours, 1);
        assert_eq!(result.minutes, 0);
        assert_eq!(result.seconds, 5);
        assert_eq!(result.milliseconds, 0);
    }

    #[test]
    fn test_add_offset_large() {
        let time = srtparse::Time {
            hours: 0,
            minutes: 0,
            seconds: 0,
            milliseconds: 100,
        };
        let result = add_offset(&time, 3661); // 1h 1m 1s
        assert_eq!(result.hours, 1);
        assert_eq!(result.minutes, 1);
        assert_eq!(result.seconds, 1);
        assert_eq!(result.milliseconds, 100);
    }

    #[test]
    fn test_add_offset_zero() {
        let time = srtparse::Time {
            hours: 2,
            minutes: 30,
            seconds: 45,
            milliseconds: 999,
        };
        let result = add_offset(&time, 0);
        assert_eq!(result.hours, 2);
        assert_eq!(result.minutes, 30);
        assert_eq!(result.seconds, 45);
        assert_eq!(result.milliseconds, 999);
    }

    #[test]
    fn test_item_to_srt_format() {
        let item = srtparse::Item {
            pos: 1,
            start_time: srtparse::Time {
                hours: 0,
                minutes: 1,
                seconds: 2,
                milliseconds: 300,
            },
            end_time: srtparse::Time {
                hours: 0,
                minutes: 1,
                seconds: 5,
                milliseconds: 600,
            },
            text: "Hello world".to_string(),
        };
        let result = item_to_srt(&item);
        assert_eq!(result, "1\n00:01:02,300 --> 00:01:05,600\nHello world\n\n");
    }

    #[test]
    fn test_item_to_srt_zero_padding() {
        let item = srtparse::Item {
            pos: 42,
            start_time: srtparse::Time {
                hours: 1,
                minutes: 2,
                seconds: 3,
                milliseconds: 4,
            },
            end_time: srtparse::Time {
                hours: 10,
                minutes: 20,
                seconds: 30,
                milliseconds: 40,
            },
            text: "Test".to_string(),
        };
        let result = item_to_srt(&item);
        assert!(result.contains("01:02:03,004"));
        assert!(result.contains("10:20:30,040"));
    }

    #[test]
    fn test_generate_result_concat() {
        let mut result1 = GenerateResult {
            generator_type: SubtitleGeneratorType::Whisper,
            subtitle_id: "1".to_string(),
            subtitle_content: vec![srtparse::Item {
                pos: 1,
                start_time: srtparse::Time {
                    hours: 0,
                    minutes: 0,
                    seconds: 0,
                    milliseconds: 0,
                },
                end_time: srtparse::Time {
                    hours: 0,
                    minutes: 0,
                    seconds: 5,
                    milliseconds: 0,
                },
                text: "First".to_string(),
            }],
        };
        let result2 = GenerateResult {
            generator_type: SubtitleGeneratorType::Whisper,
            subtitle_id: "2".to_string(),
            subtitle_content: vec![srtparse::Item {
                pos: 1,
                start_time: srtparse::Time {
                    hours: 0,
                    minutes: 0,
                    seconds: 0,
                    milliseconds: 0,
                },
                end_time: srtparse::Time {
                    hours: 0,
                    minutes: 0,
                    seconds: 3,
                    milliseconds: 0,
                },
                text: "Second".to_string(),
            }],
        };

        result1.concat(&result2, 10);
        assert_eq!(result1.subtitle_content.len(), 2);
        assert_eq!(result1.subtitle_content[1].pos, 2);
        assert_eq!(result1.subtitle_content[1].start_time.seconds, 10);
        assert_eq!(result1.subtitle_content[1].end_time.seconds, 13);
        assert_eq!(result1.subtitle_content[1].text, "Second");
    }

    #[test]
    fn test_generate_result_concat_empty_base() {
        let mut result1 = GenerateResult {
            generator_type: SubtitleGeneratorType::Whisper,
            subtitle_id: "1".to_string(),
            subtitle_content: vec![],
        };
        let result2 = GenerateResult {
            generator_type: SubtitleGeneratorType::Whisper,
            subtitle_id: "2".to_string(),
            subtitle_content: vec![srtparse::Item {
                pos: 1,
                start_time: srtparse::Time {
                    hours: 0,
                    minutes: 0,
                    seconds: 5,
                    milliseconds: 0,
                },
                end_time: srtparse::Time {
                    hours: 0,
                    minutes: 0,
                    seconds: 10,
                    milliseconds: 0,
                },
                text: "Only".to_string(),
            }],
        };

        result1.concat(&result2, 60);
        assert_eq!(result1.subtitle_content.len(), 1);
        assert_eq!(result1.subtitle_content[0].pos, 1);
        assert_eq!(result1.subtitle_content[0].start_time.minutes, 1);
        assert_eq!(result1.subtitle_content[0].start_time.seconds, 5);
    }
}
