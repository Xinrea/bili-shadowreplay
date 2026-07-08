# Whisper 后端迁移：whisper-rs → sherpa-onnx 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将本地语音识别后端从 whisper-rs (GGML) 替换为 sherpa-onnx (ONNX Runtime)，含模型自动下载和 GPU 加速支持

**Architecture:** 移除 whisper-rs/hound 依赖，新增 sherpa-onnx。新增 `model_manager` 模块管理模型下载缓存，新增 `whisper_onnx.rs` 实现 `SubtitleGenerator` trait。`ffmpeg::generate_video_subtitle()` 签名增加 `whisper_provider` 参数透传。

**Tech Stack:** sherpa-onnx 1.13, reqwest (已有), tokio (已有), bzip2 + tar (新增解压依赖)

## Global Constraints

- sherpa-onnx 版本: `1.13`
- Rust edition: 2021
- 保留 `cuda` feature 名称用于 provider 选择
- 所有异步函数使用 `async_trait`
- 遵循现有 SubtitleGenerator trait 接口
- 平台: macOS (CoreML), Windows/Linux (CPU + 可选 CUDA)

---

### Task 1: 更新 Cargo.toml 依赖

**Files:**
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Produces: `sherpa-onnx` crate available; `cuda` feature 变为条件编译标记

- [ ] **Step 1: 移除 whisper-rs 和 hound 依赖**

在 `src-tauri/Cargo.toml` 中:

删除第 51 行:
```toml
whisper-rs = "0.16.0"
```

删除第 52 行:
```toml
hound = "3.5.1"
```

删除第 74 行 `cuda` feature 的 passthrough:
```toml
cuda = ["whisper-rs/cuda"]
```
替换为:
```toml
cuda = []
```

删除第 149-154 行所有平台条件依赖:
```toml
[target.'cfg(windows)'.dependencies]
whisper-rs = { version = "0.16.0", default-features = false }

[target.'cfg(target_os = "macos")'.dependencies.whisper-rs]
version = "0.16.0"
features = ["metal"]
```

- [ ] **Step 2: 添加 sherpa-onnx 和 tar 解压依赖**

在 `[dependencies]` 中添加:
```toml
sherpa-onnx = "1.13"
bzip2 = "0.4"
tar = "0.4"
```

- [ ] **Step 3: 验证 Cargo.toml 语法**

```bash
cargo verify-project --manifest-path src-tauri/Cargo.toml
```

- [ ] **Step 4: 提交**

```bash
git add src-tauri/Cargo.toml
git commit -m "chore: replace whisper-rs with sherpa-onnx dependency"
```

---

### Task 2: 实现模型下载管理器

**Files:**
- Create: `src-tauri/src/model_manager/mod.rs`
- Modify: `src-tauri/src/main.rs` (添加 `mod model_manager;`)

**Interfaces:**
- Produces: `ModelManager::ensure_model(model_name, reporter) -> Result<ModelInfo, String>`
- Produces: `ModelManager::get_available_models() -> Vec<&'static str>`
- Produces: `ModelInfo { encoder_path: String, decoder_path: String, tokens_path: String }`

- [ ] **Step 1: 创建模块文件并添加 mod 声明**

创建 `src-tauri/src/model_manager/mod.rs`:

```rust
use crate::progress::progress_reporter::ProgressReporterTrait;
use std::path::PathBuf;

const MODEL_RELEASE_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models";

const AVAILABLE_MODELS: &[(&str, &str)] = &[
    ("tiny", "sherpa-onnx-whisper-tiny.tar.bz2"),
    ("tiny.en", "sherpa-onnx-whisper-tiny.en.tar.bz2"),
    ("base", "sherpa-onnx-whisper-base.tar.bz2"),
    ("small", "sherpa-onnx-whisper-small.tar.bz2"),
    ("large-v3", "sherpa-onnx-whisper-large-v3.tar.bz2"),
];

pub struct ModelInfo {
    pub encoder_path: String,
    pub decoder_path: String,
    pub tokens_path: String,
}

fn models_dir() -> PathBuf {
    let app_dirs = platform_dirs::AppDirs::new(Some("cn.vjoi.bili-shadowreplay"), false)
        .expect("Failed to get app dirs");
    app_dirs.data_dir.join("models")
}

fn model_dir(model_name: &str) -> PathBuf {
    models_dir().join(format!("whisper-{}", model_name))
}

fn is_model_cached(model_name: &str) -> bool {
    let dir = model_dir(model_name);
    dir.join("encoder.onnx").exists()
        && dir.join("decoder.onnx").exists()
        && dir.join("tokens.txt").exists()
}

pub fn get_available_models() -> Vec<&'static str> {
    AVAILABLE_MODELS.iter().map(|(name, _)| *name).collect()
}

pub async fn ensure_model(
    model_name: &str,
    reporter: Option<&(impl ProgressReporterTrait + 'static)>,
) -> Result<ModelInfo, String> {
    if is_model_cached(model_name) {
        let dir = model_dir(model_name);
        return Ok(ModelInfo {
            encoder_path: dir.join("encoder.onnx").to_str().unwrap().to_string(),
            decoder_path: dir.join("decoder.onnx").to_str().unwrap().to_string(),
            tokens_path: dir.join("tokens.txt").to_str().unwrap().to_string(),
        });
    }

    let (_name, filename) = AVAILABLE_MODELS
        .iter()
        .find(|(name, _)| *name == model_name)
        .ok_or_else(|| format!("Unknown model: {model_name}. Available: {:?}", get_available_models()))?;

    let url = format!("{}/{}", MODEL_RELEASE_URL, filename);
    let dir = model_dir(model_name);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create model directory: {e}"))?;

    let archive_path = dir.join(filename);

    if let Some(reporter) = reporter {
        reporter.update(&format!("正在下载模型 {}...", model_name)).await;
    }

    // Download
    let response = reqwest::get(&url)
        .await
        .map_err(|e| format!("Failed to download model {}: {e}", model_name))?;

    let total_size = response.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;
    let mut bytes = Vec::new();

    let mut stream = response.bytes_stream();
    use futures::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download error: {e}"))?;
        bytes.extend_from_slice(&chunk);
        downloaded += chunk.len() as u64;
        if let Some(reporter) = reporter {
            if total_size > 0 {
                let pct = (downloaded * 100 / total_size).min(99);
                reporter
                    .update(&format!("下载模型 {}: {}%", model_name, pct))
                    .await;
            }
        }
    }

    // Extract
    if let Some(reporter) = reporter {
        reporter.update(&format!("正在解压模型 {}...", model_name)).await;
    }

    let cursor = std::io::Cursor::new(&bytes);
    let decoder = bzip2::read::BzDecoder::new(cursor);
    let mut archive = tar::Archive::new(decoder);
    archive
        .unpack(&dir)
        .map_err(|e| format!("Failed to extract model archive: {e}"))?;

    // Clean up archive
    let _ = std::fs::remove_file(&archive_path);

    if !is_model_cached(model_name) {
        return Err(format!(
            "Model {} extraction incomplete. Expected encoder.onnx, decoder.onnx, tokens.txt in {:?}",
            model_name, dir
        ));
    }

    Ok(ModelInfo {
        encoder_path: dir.join("encoder.onnx").to_str().unwrap().to_string(),
        decoder_path: dir.join("decoder.onnx").to_str().unwrap().to_string(),
        tokens_path: dir.join("tokens.txt").to_str().unwrap().to_string(),
    })
}
```

- [ ] **Step 2: 在 main.rs 中添加模块声明**

在 `src-tauri/src/main.rs` 中，`mod migration;` 之后添加:
```rust
mod model_manager;
```

- [ ] **Step 3: 提交**

```bash
git add src-tauri/src/model_manager/mod.rs src-tauri/src/main.rs
git commit -m "feat: add model download manager for sherpa-onnx whisper models"
```

---

### Task 3: 实现 WhisperOnnx SubtitleGenerator

**Files:**
- Create: `src-tauri/src/subtitle_generator/whisper_onnx.rs`

**Interfaces:**
- Consumes: `SubtitleGenerator` trait (existing), `ModelInfo` (Task 2), `ProgressReporterTrait` (existing)
- Produces: `WhisperOnnx` struct, `new()` constructor, `generate_subtitle()` implementation

- [ ] **Step 1: 创建 whisper_onnx.rs**

创建 `src-tauri/src/subtitle_generator/whisper_onnx.rs`:

```rust
use async_trait::async_trait;
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineRecognizerResult, OfflineWhisperModelConfig,
};

use crate::{
    model_manager,
    progress::progress_reporter::ProgressReporterTrait,
    subtitle_generator::{GenerateResult, SubtitleGeneratorType},
};

use super::SubtitleGenerator;

pub struct WhisperOnnx {
    recognizer: OfflineRecognizer,
    prompt: String,
}

/// Resolve the ONNX Runtime execution provider.
/// "auto" → platform detection; otherwise pass through.
fn resolve_provider(configured: &str) -> String {
    if configured != "auto" {
        return configured.to_string();
    }
    #[cfg(target_os = "macos")]
    {
        "coreml".to_string()
    }
    #[cfg(not(target_os = "macos"))]
    {
        if cfg!(feature = "cuda") {
            "cuda".to_string()
        } else {
            "cpu".to_string()
        }
    }
}

pub async fn new(
    model_name: &str,
    provider: &str,
    prompt: &str,
    reporter: Option<&(impl ProgressReporterTrait + 'static)>,
) -> Result<WhisperOnnx, String> {
    let model_info = model_manager::ensure_model(model_name, reporter).await?;

    let provider = resolve_provider(provider);

    let mut config = OfflineRecognizerConfig::default();
    config.model_config.whisper = OfflineWhisperModelConfig {
        encoder: Some(model_info.encoder_path),
        decoder: Some(model_info.decoder_path),
        language: None,
        task: Some("transcribe".to_string()),
        tail_paddings: 0,
        enable_token_timestamps: false,
        enable_segment_timestamps: true,
    };
    config.model_config.tokens = Some(model_info.tokens_path);
    config.model_config.provider = Some(provider.clone());
    config.model_config.debug = false;
    config.model_config.num_threads = num_cpus::get() as i32;

    log::info!("Creating OfflineRecognizer with provider: {}", provider);

    let recognizer = OfflineRecognizer::create(&config)
        .map_err(|e| format!("Failed to create OfflineRecognizer: {e}"))?;

    Ok(WhisperOnnx {
        recognizer,
        prompt: prompt.to_string(),
    })
}

fn sherpa_result_to_srt_items(
    result: &OfflineRecognizerResult,
) -> Result<Vec<srtparse::Item>, String> {
    let mut items = Vec::new();

    // Use timestamps if available; otherwise treat the whole text as one segment
    if let Some(tokens) = &result.tokens {
        // Group tokens into segments by timestamp proximity
        let mut current_text = String::new();
        let mut segment_start: Option<f64> = None;
        let mut segment_end: f64 = 0.0;

        for token in tokens {
            if segment_start.is_none() {
                segment_start = Some(token.start);
            }
            current_text.push_str(&token.text);
            segment_end = token.start + token.duration; // estimate end

            // Simple heuristic: break at punctuation
            if token.text.ends_with('.')
                || token.text.ends_with('。')
                || token.text.ends_with('!')
                || token.text.ends_with('！')
                || token.text.ends_with('?')
                || token.text.ends_with('？')
                || token.text.ends_with(',')
                || token.text.ends_with('，')
            {
                let start = segment_start.unwrap_or(0.0);
                items.push(srtparse::Item {
                    pos: items.len() + 1,
                    start_time: seconds_to_srt_time(start),
                    end_time: seconds_to_srt_time(segment_end),
                    text: current_text.trim().to_string(),
                });
                current_text = String::new();
                segment_start = None;
            }
        }

        // Flush remaining text
        if !current_text.trim().is_empty() {
            let start = segment_start.unwrap_or(0.0);
            items.push(srtparse::Item {
                pos: items.len() + 1,
                start_time: seconds_to_srt_time(start),
                end_time: seconds_to_srt_time(segment_end),
                text: current_text.trim().to_string(),
            });
        }
    } else {
        // Fallback: no token timestamps, treat as single segment
        items.push(srtparse::Item {
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
                seconds: 1,
                milliseconds: 0,
            },
            text: result.text.clone(),
        });
    }

    Ok(items)
}

fn seconds_to_srt_time(seconds: f64) -> srtparse::Time {
    let total_ms = (seconds * 1000.0).round() as u64;
    let hours = (total_ms / 3_600_000) as u64;
    let minutes = ((total_ms % 3_600_000) / 60_000) as u64;
    let secs = ((total_ms % 60_000) / 1000) as u64;
    let millis = (total_ms % 1000) as u32;
    srtparse::Time {
        hours,
        minutes,
        seconds: secs,
        milliseconds: millis,
    }
}

#[async_trait]
impl SubtitleGenerator for WhisperOnnx {
    async fn generate_subtitle(
        &self,
        reporter: Option<&(impl ProgressReporterTrait + 'static)>,
        audio_path: &std::path::Path,
        _language_hint: &str,
    ) -> Result<GenerateResult, String> {
        log::info!("Generating subtitle for {:?}", audio_path);
        let start_time = std::time::Instant::now();

        if let Some(reporter) = reporter {
            reporter.update("加载音频中").await;
        }

        let wave = sherpa_onnx::Wave::read(audio_path.to_str().unwrap())
            .map_err(|e| format!("Failed to read audio file: {e}"))?;

        let stream = self.recognizer.create_stream();
        stream.accept_waveform(wave.sample_rate(), wave.samples());

        if let Some(reporter) = reporter {
            reporter.update("生成字幕中").await;
        }

        self.recognizer
            .decode(&stream)
            .map_err(|e| format!("Whisper decode failed: {e}"))?;

        let result = stream
            .get_result()
            .ok_or_else(|| "No recognition result".to_string())?;

        log::info!(
            "Time taken: {} seconds",
            start_time.elapsed().as_secs_f64()
        );

        let subtitle_content =
            sherpa_result_to_srt_items(&result).map_err(|e| format!("Failed to parse result: {e}"))?;

        Ok(GenerateResult {
            generator_type: SubtitleGeneratorType::Whisper,
            subtitle_id: String::new(),
            subtitle_content,
        })
    }
}
```

- [ ] **Step 2: 添加 num_cpus 依赖用于线程数检测**

在 `src-tauri/Cargo.toml` 的 `[dependencies]` 中添加:
```toml
num_cpus = "1.16"
```

- [ ] **Step 3: 验证编译**

```bash
cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | head -50
```

- [ ] **Step 4: 提交**

```bash
git add src-tauri/src/subtitle_generator/whisper_onnx.rs src-tauri/Cargo.toml
git commit -m "feat: add WhisperOnnx SubtitleGenerator using sherpa-onnx"
```

---

### Task 4: 更新 subtitle_generator 模块声明

**Files:**
- Modify: `src-tauri/src/subtitle_generator/mod.rs`

**Interfaces:**
- Produces: `whisper_onnx` module publicly accessible

- [ ] **Step 1: 替换模块声明**

在 `src-tauri/src/subtitle_generator/mod.rs` 中，将第 7 行:
```rust
pub mod whisper_cpp;
```
替换为:
```rust
pub mod whisper_onnx;
```

- [ ] **Step 2: 验证编译**

```bash
cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | head -30
```
预期: 会有一些 "unused import" 警告（ffmpeg/mod.rs 还在引用 whisper_cpp），这是正常的，在 Task 8 中修复。

- [ ] **Step 3: 提交**

```bash
git add src-tauri/src/subtitle_generator/mod.rs
git commit -m "refactor: replace whisper_cpp module with whisper_onnx"
```

---

### Task 5: 添加 whisper_provider 配置字段

**Files:**
- Modify: `src-tauri/src/config.rs`

**Interfaces:**
- Produces: `Config.whisper_provider: String` (默认 `"auto"`)
- Consumes: `serde` Serialize/Deserialize (existing)

- [ ] **Step 1: 添加 whisper_provider 字段到 Config struct**

在 `src-tauri/src/config.rs` 的 `Config` struct 中，`whisper_language` 之后添加:
```rust
#[serde(default = "default_whisper_provider")]
pub whisper_provider: String,
```

- [ ] **Step 2: 添加默认值函数**

在 `default_whisper_language()` 函数附近添加:
```rust
fn default_whisper_provider() -> String {
    "auto".to_string()
}
```

- [ ] **Step 3: 在 Config::load() 默认构造中添加字段**

在 `src-tauri/src/config.rs` 的 `Config::load()` 方法中，默认 Config 构造体添加:
```rust
whisper_provider: default_whisper_provider(),
```
插入到 `whisper_language: default_whisper_language(),` 行之后。

- [ ] **Step 4: 验证编译**

```bash
cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | head -30
```

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/config.rs
git commit -m "feat: add whisper_provider config field"
```

---

### Task 6: 添加 update_whisper_provider Tauri command

**Files:**
- Modify: `src-tauri/src/handlers/config.rs`

**Interfaces:**
- Produces: `update_whisper_provider` Tauri command

- [ ] **Step 1: 在 handlers/config.rs 末尾添加 command**

在 `src-tauri/src/handlers/config.rs` 的 `update_powerlive_key` 之后添加:

```rust
#[cfg_attr(feature = "gui", tauri::command)]
pub async fn update_whisper_provider(
    state: state_type!(),
    whisper_provider: String,
) -> Result<(), ()> {
    log::info!("Updating whisper provider to {whisper_provider}");
    state.config.write().await.whisper_provider = whisper_provider;
    state.config.write().await.save();
    Ok(())
}
```

- [ ] **Step 2: 验证编译**

```bash
cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | head -30
```

- [ ] **Step 3: 提交**

```bash
git add src-tauri/src/handlers/config.rs
git commit -m "feat: add update_whisper_provider command"
```

---

### Task 7: 注册新 command 到 main.rs

**Files:**
- Modify: `src-tauri/src/main.rs`

**Interfaces:**
- Consumes: `crate::handlers::config::update_whisper_provider` (Task 6)

- [ ] **Step 1: 在 invoke_handler 列表中添加**

在 `src-tauri/src/main.rs` 的 `setup_invoke_handlers()` 函数中，`crate::handlers::config::update_powerlive_key,` 之后添加:
```rust
crate::handlers::config::update_whisper_provider,
```

- [ ] **Step 2: 验证编译**

```bash
cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | head -30
```

- [ ] **Step 3: 提交**

```bash
git add src-tauri/src/main.rs
git commit -m "feat: register update_whisper_provider command"
```

---

### Task 8: 更新 ffmpeg 调度层接入新后端

**Files:**
- Modify: `src-tauri/src/ffmpeg/mod.rs`

**Interfaces:**
- Modify: `generate_video_subtitle()` 签名增加 `whisper_provider: &str` 参数
- Consumes: `whisper_onnx::new()` (Task 3), `SubtitleGenerator` trait (existing)

- [ ] **Step 1: 修改函数签名和调用处**

修改 `src-tauri/src/ffmpeg/mod.rs` 中的 `generate_video_subtitle` 函数签名，在第 666 行 `whisper_prompt: &str,` 后添加:
```rust
whisper_provider: &str,
```

将第 677 行:
```rust
if let Ok(generator) = whisper_cpp::new(Path::new(&whisper_model), whisper_prompt).await
```
替换为:
```rust
if let Ok(generator) = whisper_onnx::new(&whisper_model, whisper_provider, whisper_prompt, reporter).await
```

将第 11-13 行 import 从:
```rust
use crate::subtitle_generator::{
    whisper_cpp, GenerateResult, SubtitleGenerator, SubtitleGeneratorType,
};
```
修改为:
```rust
use crate::subtitle_generator::{
    whisper_onnx, GenerateResult, SubtitleGenerator, SubtitleGeneratorType,
};
```

- [ ] **Step 2: 更新 handlers/video.rs 调用处**

在 `src-tauri/src/handlers/video.rs` 第 843-848 行区域，读取 config 后添加:
```rust
let whisper_provider = config.whisper_provider.clone();
```

将第 855-864 行的 `ffmpeg::generate_video_subtitle(...)` 调用，在 `language_hint,` 之前添加:
```rust
&whisper_provider,
```

- [ ] **Step 3: 更新 recorder_manager.rs 调用处**

在 `src-tauri/src/recorder_manager.rs` 第 1340-1349 行的 `crate::ffmpeg::generate_video_subtitle(...)` 调用中，`&config.whisper_language,` 之前添加:
```rust
&config.whisper_provider,
```

- [ ] **Step 4: 验证编译**

```bash
cargo check --manifest-path src-tauri/Cargo.toml 2>&1
```
预期: 可能有关于 `hound`, `whisper_rs` 的 unused import 残留警告，在 Task 10 清理。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/ffmpeg/mod.rs src-tauri/src/handlers/video.rs src-tauri/src/recorder_manager.rs
git commit -m "refactor: wire up whisper_onnx backend in ffmpeg dispatch"
```

---

### Task 9: 清理旧文件和测试资源

**Files:**
- Delete: `src-tauri/src/subtitle_generator/whisper_cpp.rs`
- Delete: `src-tauri/tests/model/ggml-tiny-q5_1.bin` (GGML 格式，不再使用)

- [ ] **Step 1: 删除旧文件**

```bash
rm src-tauri/src/subtitle_generator/whisper_cpp.rs
rm src-tauri/tests/model/ggml-tiny-q5_1.bin
```

- [ ] **Step 2: 验证编译无错误**

```bash
cargo check --manifest-path src-tauri/Cargo.toml 2>&1
```
预期: 编译成功，无错误。之前引用 `whisper_cpp` 的 import 应该在 Task 8 中全部替换完成。

- [ ] **Step 3: 运行项目现有测试**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib 2>&1
```

- [ ] **Step 4: 提交**

```bash
git add src-tauri/src/subtitle_generator/whisper_cpp.rs src-tauri/tests/model/ggml-tiny-q5_1.bin
git commit -m "chore: remove old whisper-rs implementation and GGML test model"
```

---

### Task 10: 添加单元测试

**Files:**
- Modify: `src-tauri/src/model_manager/mod.rs` (添加测试)

- [ ] **Step 1: 添加 model_manager 单元测试**

在 `src-tauri/src/model_manager/mod.rs` 末尾追加:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_available_models() {
        let models = get_available_models();
        assert!(models.contains(&"tiny"));
        assert!(models.contains(&"base"));
        assert!(models.contains(&"small"));
        assert!(!models.is_empty());
    }

    #[test]
    fn test_unknown_model_returns_error() {
        // ensure_model with invalid name should return error (without network call
        // because the model name lookup happens first)
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(ensure_model("nonexistent-model", None::<&crate::progress::progress_reporter::ProgressReporter>));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown model"));
    }

    #[test]
    fn test_models_dir_returns_path() {
        let dir = models_dir();
        assert!(dir.to_string_lossy().contains("models"));
    }
}
```

- [ ] **Step 2: 添加 whisper_onnx provider 测试**

在 `src-tauri/src/subtitle_generator/whisper_onnx.rs` 末尾追加:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seconds_to_srt_time_zero() {
        let time = seconds_to_srt_time(0.0);
        assert_eq!(time.hours, 0);
        assert_eq!(time.minutes, 0);
        assert_eq!(time.seconds, 0);
        assert_eq!(time.milliseconds, 0);
    }

    #[test]
    fn test_seconds_to_srt_time_one_hour() {
        let time = seconds_to_srt_time(3661.5);
        assert_eq!(time.hours, 1);
        assert_eq!(time.minutes, 1);
        assert_eq!(time.seconds, 1);
        assert_eq!(time.milliseconds, 500);
    }

    #[test]
    fn test_seconds_to_srt_time_with_millis() {
        let time = seconds_to_srt_time(125.789);
        assert_eq!(time.hours, 0);
        assert_eq!(time.minutes, 2);
        assert_eq!(time.seconds, 5);
        assert_eq!(time.milliseconds, 789);
    }

    #[test]
    fn test_resolve_provider_auto_macos() {
        let provider = resolve_provider("auto");
        #[cfg(target_os = "macos")]
        assert_eq!(provider, "coreml");
        #[cfg(not(target_os = "macos"))]
        assert!(provider == "cpu" || provider == "cuda");
    }

    #[test]
    fn test_resolve_provider_explicit() {
        assert_eq!(resolve_provider("cpu"), "cpu");
        assert_eq!(resolve_provider("cuda"), "cuda");
    }
}
```

- [ ] **Step 3: 运行测试**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib 2>&1
```
预期: 所有测试通过。

- [ ] **Step 4: 提交**

```bash
git add src-tauri/src/model_manager/mod.rs src-tauri/src/subtitle_generator/whisper_onnx.rs
git commit -m "test: add unit tests for model manager and whisper_onnx"
```

---

### Task 11: 验证构建（headless + gui）

**Files:** N/A (验证步骤)

- [ ] **Step 1: 编译 headless feature**

```bash
cargo check --manifest-path src-tauri/Cargo.toml --features headless 2>&1
```
预期: 编译成功。

- [ ] **Step 2: 编译 gui default features (macOS)**

```bash
cargo check --manifest-path src-tauri/Cargo.toml 2>&1
```
预期: 编译成功。

- [ ] **Step 3: 编译 CUDA feature (可选，仅验证 feature 存在)**

```bash
cargo check --manifest-path src-tauri/Cargo.toml --features cuda 2>&1 | tail -5
```
预期: 编译成功（即使没有 CUDA SDK，sherpa-onnx 在构建时只是编译 Rust 代码，运行时会 fallback）。

- [ ] **Step 4: 运行完整测试套件**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib 2>&1
```
预期: 所有非 #[ignore] 测试通过。

- [ ] **Step 5: 提交（如有 Cargo.lock 变更）**

```bash
git add src-tauri/Cargo.lock
git commit -m "chore: update Cargo.lock for sherpa-onnx migration"
```

---

### Task 12: 最终验证与文档更新

**Files:**
- Modify: `docs/getting-started/config/whisper.md` (更新文档)

- [ ] **Step 1: 更新 whisper 配置文档**

修改 `docs/getting-started/config/whisper.md`，将模型下载说明从 whisper.cpp 的 HuggingFace 链接改为 sherpa-onnx 模型名称说明:

```markdown
# Whisper 配置 (sherpa-onnx)

## 模型

项目使用 sherpa-onnx（基于 ONNX Runtime）运行 Whisper 模型。
模型会在首次使用时自动下载。

### 可用模型

| 模型名 | 大小 | 语言 |
|--------|------|------|
| `tiny` | ~75MB | 多语言 |
| `tiny.en` | ~75MB | 仅英文 |
| `base` | ~145MB | 多语言 |
| `small` | ~465MB | 多语言 |
| `large-v3` | ~2.9GB | 多语言 |

模型文件会自动下载到应用数据目录的 `models/` 文件夹中。

## 硬件加速

- **macOS**: 自动使用 CoreML 加速
- **Windows/Linux**: 默认 CPU，启用 CUDA feature 后可使用 NVIDIA GPU
```

- [ ] **Step 2: 运行 pre-commit hooks 检查全文**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib 2>&1
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check 2>&1
```

- [ ] **Step 3: 最终提交**

```bash
git add docs/getting-started/config/whisper.md
git commit -m "docs: update whisper config docs for sherpa-onnx migration"
```
