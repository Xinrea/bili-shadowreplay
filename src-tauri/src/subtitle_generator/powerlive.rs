// curl -X POST https://www.powerlive.io/api/service/subtitle \                       base at 19:00:43
//   -H "Authorization: Bearer xxxxxx" \
//   -F "format=opus" \
//   -F "sentence_max_length=10" \
//   -F "file=@/Users/xinreasuper/Desktop/shadowreplay-test/output2/[27628030][1762873698516][生日月和轴一前辈的夜夜夜谈][2025-11-11_23-35-01].opus"
use async_trait::async_trait;
use reqwest::{
    multipart::{Form, Part},
    Client,
};
use std::path::Path;

use crate::{
    progress::progress_reporter::ProgressReporterTrait,
    subtitle_generator::{GenerateResult, SubtitleGenerator, SubtitleGeneratorType},
};

const API_URL: &str = "https://www.powerlive.io/api/service/subtitle";

#[derive(Debug, Clone)]
pub struct PowerLive {
    client: Client,
    api_key: String,
}

pub async fn new(api_key: &str) -> Result<PowerLive, String> {
    let client = Client::new();
    Ok(PowerLive {
        client,
        api_key: api_key.to_string(),
    })
}

#[async_trait]
impl SubtitleGenerator for PowerLive {
    async fn generate_subtitle(
        &self,
        _reporter: Option<&(impl ProgressReporterTrait + 'static)>,
        audio_path: &Path,
        _language_hint: &str,
    ) -> Result<GenerateResult, String> {
        let audio_data = tokio::fs::read(audio_path)
            .await
            .map_err(|e| format!("Failed to read audio file: {e}"))?;

        // Get file name from path
        let file_name = audio_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        // Get file extension for proper MIME type
        let file_extension = audio_path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "opus".to_string());

        let mime_type = match file_extension.to_lowercase().as_str() {
            "opus" => "audio/opus",
            "wav" => "audio/wav",
            "mp3" => "audio/mpeg",
            "m4a" => "audio/mp4",
            "flac" => "audio/flac",
            _ => "audio/opus",
        };

        let audio_file = Part::bytes(audio_data)
            .mime_str(mime_type)
            .map_err(|e| format!("Failed to set MIME type: {e}"))?
            .file_name(file_name);

        let form = Form::new()
            .text("format", file_extension)
            .text("sentence_max_length", "10")
            .part("file", audio_file);

        let response = self
            .client
            .post(API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("Failed to send request: {e}"))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            log::error!("API request failed: {error_text}");
            return Err(format!("Failed to generate subtitle: {error_text}"));
        }

        let body = response
            .json::<serde_json::Value>()
            .await
            .map_err(|e| format!("Failed to get response text: {e}"))?;

        // {
        //     "message": "success",
        //     "subtitle": "1\n00:00:00,000 --> 00:00:00,000\nHello, world!\n\n"
        // }

        let subtitle = body["subtitle"].as_str().ok_or("Failed to get subtitle")?;

        let subtitle_content =
            srtparse::from_str(subtitle).map_err(|e| format!("Failed to parse subtitle: {e}"))?;

        Ok(GenerateResult {
            generator_type: SubtitleGeneratorType::PowerLive,
            subtitle_id: String::new(),
            subtitle_content,
        })
    }
}
