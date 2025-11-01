use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::path::Path;
use tokio::fs;

use crate::{
    progress::progress_reporter::ProgressReporterTrait,
    subtitle_generator::{GenerateResult, SubtitleGenerator, SubtitleGeneratorType},
};

#[derive(Debug, Clone)]
pub struct WhisperOnline {
    client: Client,
    api_url: String,
    api_key: Option<String>,
    prompt: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WhisperResponse {
    segments: Vec<WhisperSegment>,
}

#[derive(Debug, Deserialize)]
struct WhisperSegment {
    start: f64,
    end: f64,
    text: String,
}

pub async fn new(
    api_url: Option<&str>,
    api_key: Option<&str>,
    prompt: Option<&str>,
) -> Result<WhisperOnline, String> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(300)) // 5 minutes timeout
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    let api_url = api_url.unwrap_or("https://api.openai.com/v1");
    let api_url = api_url.to_string() + "/audio/transcriptions";

    Ok(WhisperOnline {
        client,
        api_url: api_url.to_string(),
        api_key: api_key.map(std::string::ToString::to_string),
        prompt: prompt.map(std::string::ToString::to_string),
    })
}

#[async_trait]
impl SubtitleGenerator for WhisperOnline {
    async fn generate_subtitle(
        &self,
        reporter: Option<&impl ProgressReporterTrait>,
        audio_path: &Path,
        language_hint: &str,
    ) -> Result<GenerateResult, String> {
        log::info!("Generating subtitle online for {:?}", audio_path);
        let start_time = std::time::Instant::now();

        // Read audio file
        if let Some(reporter) = reporter {
            reporter.update("读取音频文件中").await;
        }
        let audio_data = fs::read(audio_path)
            .await
            .map_err(|e| format!("Failed to read audio file: {e}"))?;

        // Get file extension for proper MIME type
        let file_extension = audio_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("wav");

        let mime_type = match file_extension.to_lowercase().as_str() {
            "wav" => "audio/wav",
            "mp3" => "audio/mpeg",
            "m4a" => "audio/mp4",
            "flac" => "audio/flac",
            _ => "audio/wav",
        };

        // Build form data with proper file part
        let file_part = reqwest::multipart::Part::bytes(audio_data)
            .mime_str(mime_type)
            .map_err(|e| format!("Failed to set MIME type: {e}"))?
            .file_name(
                audio_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            );

        let mut form = reqwest::multipart::Form::new()
            .part("file", file_part)
            .text("model", "whisper-1")
            .text("response_format", "verbose_json")
            .text("temperature", "0.0");

        form = form.text("language", language_hint.to_string());

        if let Some(prompt) = self.prompt.clone() {
            form = form.text("prompt", prompt);
        }

        // Build HTTP request
        let mut req_builder = self.client.post(&self.api_url);

        if let Some(api_key) = &self.api_key {
            req_builder = req_builder.header("Authorization", format!("Bearer {api_key}"));
        }

        if let Some(reporter) = reporter {
            reporter.update("上传音频中").await;
        }
        let response = req_builder
            .timeout(std::time::Duration::from_secs(3 * 60)) // 3 minutes timeout
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            log::error!("API request failed with status {status}: {error_text}");
            return Err(format!(
                "API request failed with status {status}: {error_text}"
            ));
        }

        // Get the raw response text first for debugging
        let response_text = response
            .text()
            .await
            .map_err(|e| format!("Failed to get response text: {e}"))?;

        // Try to parse as JSON
        let whisper_response: WhisperResponse =
            serde_json::from_str(&response_text).map_err(|e| {
                println!("{response_text}");
                log::error!("Failed to parse JSON response. Raw response: {response_text}");
                format!("Failed to parse response: {e}")
            })?;

        // Generate SRT format subtitle
        let mut subtitle = String::new();
        for (i, segment) in whisper_response.segments.iter().enumerate() {
            let format_time = |timestamp: f64| {
                let hours = (timestamp / 3600.0).floor();
                let minutes = ((timestamp - hours * 3600.0) / 60.0).floor();
                let seconds = (timestamp - hours * 3600.0 - minutes * 60.0).floor();
                let milliseconds = ((timestamp - hours * 3600.0 - minutes * 60.0 - seconds)
                    * 1000.0)
                    .floor() as u32;
                format!("{hours:02}:{minutes:02}:{seconds:02},{milliseconds:03}")
            };

            let line = format!(
                "{}\n{} --> {}\n{}\n\n",
                i + 1,
                format_time(segment.start),
                format_time(segment.end),
                segment.text.trim(),
            );

            subtitle.push_str(&line);
        }

        log::info!("Time taken: {} seconds", start_time.elapsed().as_secs_f64());

        let subtitle_content =
            srtparse::from_str(&subtitle).map_err(|e| format!("Failed to parse subtitle: {e}"))?;

        Ok(GenerateResult {
            generator_type: SubtitleGeneratorType::WhisperOnline,
            subtitle_id: String::new(),
            subtitle_content,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    // Mock reporter for testing
    #[derive(Clone)]
    struct MockReporter {}

    #[async_trait]
    impl ProgressReporterTrait for MockReporter {
        async fn update(&self, message: &str) {
            println!("Mock update: {message}");
        }

        async fn finish(&self, success: bool, message: &str) {
            if success {
                println!("Mock finish: {message}");
            } else {
                println!("Mock error: {message}");
            }
        }
    }

    impl MockReporter {
        fn new() -> Self {
            MockReporter {}
        }
    }

    #[tokio::test]
    async fn test_create_whisper_online() {
        let result = new(Some("https://api.openai.com/v1"), Some("test-key"), None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[ignore = "requres api key"]
    async fn test_generate_subtitle() {
        let result = new(Some("https://api.openai.com/v1"), Some("sk-****"), None).await;
        assert!(result.is_ok());
        let result = result.unwrap();
        let result = result
            .generate_subtitle(
                Some(&MockReporter::new()),
                Path::new("tests/audio/test.wav"),
                "auto",
            )
            .await;
        println!("{result:?}");
        assert!(result.is_ok());
        let result = result.unwrap();
        println!("{:?}", result.subtitle_content);
    }
}
