# Whisper 后端迁移：whisper-rs → sherpa-onnx

**日期:** 2026-07-05
**状态:** 待审核

---

## 目标

将本地语音识别后端从 `whisper-rs` (whisper.cpp GGML) 替换为 `sherpa-onnx` (ONNX Runtime)，解决 whisper-rs 编译和性能问题，同时保持所有现有功能。

## 非目标

- 不修改在线 Whisper API (`whisper_online.rs`) 和 PowerLive (`powerlive.rs`)
- 不修改前端 UI（本次只改后端）
- 不删除用户已有的 GGML 模型文件（但不继续使用）

---

## 架构变更

### 依赖变更 (`src-tauri/Cargo.toml`)

**移除:**
- `whisper-rs = "0.16.0"` 及所有平台条件依赖
- `hound = "3.5.1"` (sherpa-onnx 内置 `Wave::read()`)

**新增:**
- `sherpa-onnx = "1.13"`

**保留但语义变更:**
- `cuda` feature：从 `whisper-rs/cuda` passthrough 改为条件编译标记，用于 provider 选择

### 文件变更清单

| 操作 | 文件 | 说明 |
|---|---|---|
| 删除 | `src/subtitle_generator/whisper_cpp.rs` | 旧 whisper-rs 实现 |
| 新增 | `src/subtitle_generator/whisper_onnx.rs` | 新 sherpa-onnx 实现 |
| 新增 | `src/model_manager/mod.rs` | 模型下载与缓存管理 |
| 修改 | `src/subtitle_generator/mod.rs` | 模块声明：`whisper_cpp` → `whisper_onnx` |
| 修改 | `src/ffmpeg/mod.rs` | 引用更新：`whisper_cpp` → `whisper_onnx` |
| 修改 | `src/config.rs` | 新增 `whisper_provider` 字段 |
| 修改 | `src/handlers/config.rs` | 新增 provider 相关 Tauri command |
| 修改 | `src/main.rs` | 注册新 command |
| 修改 | `Cargo.toml` | 依赖变更 |

---

## 组件设计

### 1. ModelManager (`src/model_manager/mod.rs`)

负责 Whisper ONNX 模型的下载、缓存和路径解析。

**公开接口:**
```rust
pub struct ModelInfo {
    pub encoder_path: String,
    pub decoder_path: String,
    pub tokens_path: String,
}

pub async fn ensure_model(model_name: &str, reporter: Option<&impl ProgressReporterTrait>)
    -> Result<ModelInfo, String>;
```

**模型注册表:**

| 键名 | 下载文件名 | 大小 | 语言 |
|---|---|---|---|
| `tiny` | `sherpa-onnx-whisper-tiny.tar.bz2` | ~75MB | 多语言 |
| `tiny.en` | `sherpa-onnx-whisper-tiny.en.tar.bz2` | ~75MB | 仅英文 |
| `base` | `sherpa-onnx-whisper-base.tar.bz2` | ~145MB | 多语言 |
| `small` | `sherpa-onnx-whisper-small.tar.bz2` | ~465MB | 多语言 |
| `large-v3` | `sherpa-onnx-whisper-large-v3.tar.bz2` | ~2.9GB | 多语言 |

下载源：`https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/`

缓存路径：`{app_data_dir}/models/whisper-{name}/`

**内部逻辑:**
1. 检查缓存目录是否存在且包含 3 个必需文件（`encoder.onnx`, `decoder.onnx`, `tokens.txt`）
2. 缺失 → 用 `reqwest` 流式下载 `.tar.bz2`，通过 reporter 回调进度
3. 解压 `tar.bz2` 到缓存目录
4. 删除压缩包
5. 返回 `ModelInfo`

错误处理：
- 网络错误 → 返回可读错误信息，建议检查网络
- 解压失败 → 清理不完整缓存，返回错误
- 磁盘空间不足 → 返回带建议的错误信息

### 2. WhisperOnnx (`src/subtitle_generator/whisper_onnx.rs`)

实现 `SubtitleGenerator` trait 的新后端。

**结构体:**
```rust
pub struct WhisperOnnx {
    recognizer: OfflineRecognizer,
    prompt: String,
}
```

**构造函数 `new()`:**
```rust
pub async fn new(
    model_name: &str,
    provider: &str,
    prompt: &str,
    reporter: Option<&impl ProgressReporterTrait>,
) -> Result<WhisperOnnx, String>
```
1. 调用 `ModelManager::ensure_model(model_name, reporter)` 获取模型路径
2. 构建 `OfflineRecognizerConfig`:
   - `model_config.whisper`: encoder/decoder 路径、language 默认 auto、task=transcribe
   - `model_config.tokens`: tokens 路径
   - `model_config.provider`: 传入的 provider
   - `model_config.num_threads`: 自动（0 或 cpu 核心数）
   - `enable_segment_timestamps: true`（SRT 字幕需要！）
3. `OfflineRecognizer::create(&config)` 创建识别器
4. 返回 `WhisperOnnx` 实例

**`generate_subtitle()` 实现:**
1. `Wave::read(audio_path)` 加载 WAV（替代 hound + 手动转换）
2. `stream.accept_waveform(sample_rate, &samples)` 送入音频
3. `recognizer.decode(&stream)` 执行推理
4. `stream.get_result()` 获取带时间戳的结果
5. 将 sherpa-onnx 的 segments 转换为 `srtparse::Item` 格式
6. 返回 `GenerateResult`

**与 whisper-rs 的关键区别:**
- 不再需要 `Arc<RwLock<>>` — `OfflineRecognizer` 内部已处理线程安全
- 不再需要 `convert_integer_to_float_audio` / `convert_stereo_to_mono_audio`
- 不再需要 `create_state()` / `WhisperContext` 管理生命周期
- 时间戳格式可能不同（sherpa-onnx 使用秒，whisper-rs 使用毫秒×100）

### 3. GPU Provider 选择

```rust
fn resolve_provider(configured: &str) -> String {
    if configured != "auto" {
        return configured.to_string();
    }
    // 自动检测
    if cfg!(target_os = "macos") {
        "coreml".to_string()
    } else if cfg!(feature = "cuda") {
        "cuda".to_string()
    } else {
        "cpu".to_string()
    }
}
```

### 4. 配置变更 (`config.rs`)

新增字段：
```rust
pub whisper_provider: String,  // 默认 "auto"，可选 "cpu" / "cuda" / "coreml"
```

保留字段（语义不变）：
- `whisper_model` — 含义变为「模型名称」，如 `"tiny"`、`"base"`
- `whisper_prompt` — 初始终提示词，保持不变
- `whisper_language` — 语言提示，不变

### 5. ffmpeg 调度层变更 (`ffmpeg/mod.rs`)

`generate_video_subtitle()` 中 whisper 分支调整：
- 模型路径 → 模型名称（从 config 传入）
- 调用 `whisper_onnx::new(model_name, provider, prompt, reporter)` 替代 `whisper_cpp::new(model_path, prompt)`
- 其余流程不变（音频分块、逐块处理、合并结果）

---

## 测试策略

### 单元测试

| 测试 | 位置 | 内容 |
|---|---|---|
| model_registry | `model_manager/mod.rs` | 验证注册表模型名有效 |
| model_cache_check | `model_manager/mod.rs` | mock 文件系统，验证缓存检测逻辑 |
| provider_resolution | `whisper_onnx.rs` | 验证 auto/cpu/cuda/coreml 逻辑 |
| srt_format_conversion | `whisper_onnx.rs` | sherpa-onnx segment → srtparse::Item |

### 集成测试

- 下载 `tiny` 模型并运行完整字幕生成（使用测试音频 `tests/audio/test.wav`）
- 标记 `#[ignore]` 避免 CI 每次都下载模型

### 回归测试

- `mod.rs` 中的 `SubtitleGeneratorType` 相关测试保持不变
- `ffmpeg/mod.rs` 中的错误处理测试保持不变

---

## 错误处理

| 场景 | 处理方式 |
|---|---|
| 模型未下载/网络不可用 | 返回 `"模型 {name} 下载失败: {details}"` |
| 模型文件损坏 | 检测后提示删除缓存重试 |
| ONNX Runtime 初始化失败 | 返回 provider 相关建议（如 CUDA 用户检查驱动） |
| 音频文件无效格式 | `Wave::read()` 返回的错误直接透传 |
| 推理失败 (如 OOM) | 建议使用更小模型 |

---

## 向后兼容

- 旧 `whisper_model` 配置值（如 `"whisper_model.bin"`）不再有效
- 首次启动时需要用户重新选择模型（或检测到旧配置时自动迁移为 `"base"`）
- 旧 `.bin` 模型文件不自动删除，用户可手动清理

---

## 前置条件

- sherpa-onnx crate 需要联网下载预编译 ONNX Runtime 原生库（首次 `cargo build` 时）
- 模型下载也需要首次联网

---

## spec 自审

- [x] 无 TODO / TBD 占位符
- [x] 架构描述与文件变更清单一致
- [x] 所有公开接口有明确的签名和职责
- [x] 错误场景覆盖完整
- [x] 测试策略覆盖单元/集成/回归
