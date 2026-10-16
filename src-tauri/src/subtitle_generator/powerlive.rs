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

#[derive(Clone)]
pub struct PowerLive<'a> {
    client: Client,
    db: &'a crate::database::Database,
    api_base: String,
}

pub async fn new(db: &crate::database::Database) -> Result<PowerLive<'_>, String> {
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())?;
    Ok(PowerLive {
        client,
        db,
        api_base: crate::powerlive::API_BASE.to_string(),
    })
}

#[async_trait]
impl SubtitleGenerator for PowerLive<'_> {
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

        let (response, _) =
            crate::powerlive::authenticated_request(self.db, &self.api_base, |token| {
                // Multipart bodies must be rebuilt to retry a rejected access token once.
                let audio_file = Part::bytes(audio_data.clone())
                    .mime_str(mime_type)
                    .map_err(|e| format!("Failed to set MIME type: {e}"))?
                    .file_name(file_name.clone());
                let form = Form::new()
                    .text("format", file_extension.clone())
                    .text("sentence_max_length", "10")
                    .part("file", audio_file);
                Ok(self
                    .client
                    .post(format!("{}/service/subtitle", self.api_base))
                    .bearer_auth(token)
                    .multipart(form))
            })
            .await?;

        if !response.status().is_success() {
            let status = response.status();
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
    use axum::{
        extract::{Form as AxumForm, Multipart, State},
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::post,
        Json, Router,
    };
    use std::{
        collections::HashMap,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };

    #[tokio::test]
    async fn retries_unauthorized_upload_with_refreshed_bearer_and_same_audio() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let router = Router::new().route("/service/subtitle", post(|State(calls): State<Arc<AtomicUsize>>, headers: HeaderMap, mut form: Multipart| async move {
            calls.fetch_add(1, Ordering::SeqCst);
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
            if headers["authorization"] == "Bearer test-account-token" {
                StatusCode::UNAUTHORIZED.into_response()
            } else {
                assert_eq!(headers["authorization"], "Bearer new-access");
                Json(serde_json::json!({"subtitle": "1\n00:00:00,000 --> 00:00:01,000\n测试字幕\n\n"})).into_response()
            }
        })).route("/oauth/token", post(|AxumForm(form): AxumForm<HashMap<String, String>>| async move {
            assert_eq!(form["refresh_token"], "refresh-token");
            Json(serde_json::json!({"access_token":"new-access", "refresh_token":"rotated-refresh", "expires_in":900, "token_type":"Bearer"}))
        })).with_state(calls.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let directory = tempfile::tempdir().unwrap();
        let audio = directory.path().join("audio.opus");
        tokio::fs::write(&audio, b"test-audio").await.unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for migration in crate::get_migrations() {
            sqlx::raw_sql(migration.sql).execute(&pool).await.unwrap();
        }
        let db = crate::database::Database::new(
            pool,
            crate::database::credentials::CredentialCipher::new(&[0; 32]).unwrap(),
            crate::events::EventBus::new(),
        );
        db.save_powerlive_session(&serde_json::json!({"access_token":"test-account-token", "refresh_token":"refresh-token", "expires_at":chrono::Utc::now().timestamp()+900, "name":"Alice", "subject":"user-1"}).to_string()).await.unwrap();
        let mut generator = new(&db).await.unwrap();
        generator.api_base = endpoint;
        let result = generator
            .generate_subtitle(
                None::<&crate::progress::progress_reporter::ProgressReporter>,
                &audio,
                "zh",
            )
            .await
            .unwrap();
        assert_eq!(result.subtitle_content.len(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        server.abort();
    }
}
