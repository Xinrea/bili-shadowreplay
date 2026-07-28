use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::progress::progress_reporter::ProgressReporterTrait;

const SILERO_VAD_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx";
const SILERO_VAD_FILENAME: &str = "silero_vad.onnx";

static DOWNLOAD_LOCK: OnceLock<async_std::sync::Mutex<()>> = OnceLock::new();

pub async fn ensure_silero_vad_model(
    cache_dir: &Path,
    reporter: Option<&(impl ProgressReporterTrait + 'static)>,
) -> Result<PathBuf, String> {
    let model_dir = cache_dir.join("models").join("silero-vad");
    let model_path = model_dir.join(SILERO_VAD_FILENAME);
    if model_path.is_file() {
        return Ok(model_path);
    }

    let lock = DOWNLOAD_LOCK.get_or_init(|| async_std::sync::Mutex::new(()));
    let _guard = lock.lock().await;
    if model_path.is_file() {
        return Ok(model_path);
    }

    async_std::fs::create_dir_all(&model_dir)
        .await
        .map_err(|error| format!("Failed to create Silero VAD model directory: {error}"))?;
    if let Some(reporter) = reporter {
        reporter.update("下载 Silero VAD 模型中").await;
    }

    let response = reqwest::Client::new()
        .get(SILERO_VAD_URL)
        .send()
        .await
        .map_err(|error| format!("Failed to download Silero VAD model: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Failed to download Silero VAD model: HTTP {}",
            response.status()
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("Failed to read Silero VAD model response: {error}"))?;
    let temporary_path = model_path.with_extension("download");
    async_std::fs::write(&temporary_path, bytes)
        .await
        .map_err(|error| format!("Failed to write Silero VAD model: {error}"))?;
    if model_path.exists() {
        async_std::fs::remove_file(&model_path)
            .await
            .map_err(|error| format!("Failed to replace Silero VAD model: {error}"))?;
    }
    async_std::fs::rename(&temporary_path, &model_path)
        .await
        .map_err(|error| format!("Failed to finalize Silero VAD model: {error}"))?;

    Ok(model_path)
}
