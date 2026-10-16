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

const API_URL: &str = "https://api.powerlive.io/service/subtitle";

#[derive(Clone)]
pub struct PowerLive {
    client: Client,
    access_token: String,
    api_url: String,
}

pub async fn new(access_token: &str) -> Result<PowerLive, String> {
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())?;
    Ok(PowerLive {
        client,
        access_token: access_token.to_string(),
        api_url: API_URL.to_string(),
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
            .map(str::to_ascii_lowercase)
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
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("Failed to send request: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err("PowerLive 登录已失效，请在设置中重新登录".into());
            }
            let body = response
                .json::<serde_json::Value>()
                .await
                .unwrap_or_default();
            let message = body["message"].as_str().unwrap_or("PowerLive 字幕生成失败");
            return Err(format!("PowerLive ({status}): {message}"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Multipart, http::HeaderMap, routing::post, Json, Router};

    #[tokio::test]
    async fn uploads_audio_with_account_bearer_and_parses_subtitles() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/service/subtitle", listener.local_addr().unwrap());
        let router = Router::new().route("/service/subtitle", post(|headers: HeaderMap, mut form: Multipart| async move {
            assert_eq!(headers["authorization"], "Bearer test-account-token");
            let mut names = Vec::new();
            while let Some(field) = form.next_field().await.unwrap() {
                let name = field.name().unwrap().to_string();
                match name.as_str() {
                    "format" => assert_eq!(field.text().await.unwrap(), "opus"),
                    "sentence_max_length" => assert_eq!(field.text().await.unwrap(), "10"),
                    "file" => {
                        assert_eq!(field.file_name(), Some("audio.opus"));
                        assert_eq!(field.content_type(), Some("audio/opus"));
                        assert_eq!(field.bytes().await.unwrap().as_ref(), b"test-audio");
                    }
                    _ => panic!("unexpected field"),
                }
                names.push(name);
            }
            assert_eq!(names.len(), 3);
            Json(serde_json::json!({"subtitle": "1\n00:00:00,000 --> 00:00:01,000\n测试字幕\n\n"}))
        }));
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let directory = tempfile::tempdir().unwrap();
        let audio = directory.path().join("audio.opus");
        tokio::fs::write(&audio, b"test-audio").await.unwrap();
        let mut generator = new("test-account-token").await.unwrap();
        generator.api_url = endpoint;
        let result = generator
            .generate_subtitle(
                None::<&crate::progress::progress_reporter::ProgressReporter>,
                &audio,
                "zh",
            )
            .await
            .unwrap();
        assert_eq!(result.subtitle_content.len(), 1);
        server.abort();
    }
}
