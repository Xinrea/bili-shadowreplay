# Whisper 配置

要使用 AI 字幕识别功能，需要在设置页面配置 Whisper。目前可以选择使用本地运行 Whisper 模型，或是使用在线的 Whisper 服务（通常需要付费获取 API Key）。

> [!NOTE]
> 其实有许多更好的中文字幕识别解决方案，但是这类服务通常需要将文件上传到对象存储后异步处理，考虑到实现的复杂度，选择了使用本地运行 Whisper 模型或是使用在线的 Whisper 服务，在请求返回时能够直接获取字幕生成结果。

## 本地运行 Whisper 模型

![WhisperLocal](/images/whisper_local.png)

本地 Whisper 字幕生成使用 sherpa-onnx（基于 ONNX Runtime）运行 Whisper 模型。模型会在首次使用时自动下载，无需手动下载模型文件。

### 可用模型

| 模型名 | 大小 | 语言 | 说明 |
|--------|------|------|------|
| `tiny` | ~75MB | 多语言 | 速度最快，精度较低 |
| `tiny.en` | ~75MB | 仅英文 | 英文优化版本 |
| `base` | ~145MB | 多语言 | 速度与精度平衡 |
| `small` | ~465MB | 多语言 | 较高质量 |
| `large-v3` | ~2.9GB | 多语言 | 最高精度，速度最慢 |

模型文件会自动下载到应用数据目录的 `models/` 文件夹中。

### 硬件加速

- **macOS**: 自动使用 CoreML 加速（Apple Silicon / Intel）
- **Windows / Linux**: 默认使用 CPU，启用 CUDA 后可支持 NVIDIA GPU 加速

## 使用在线 Whisper 服务

![WhisperOnline](/images/whisper_online.png)

如果需要使用在线的 Whisper 服务进行字幕生成，可以在设置中切换为在线 Whisper，并配置好 API Key。提供 Whisper 服务的平台并非只有
OpenAI 一家，许多云服务平台也提供 Whisper 服务。

## 字幕识别质量的调优

目前在设置中支持设置 Whisper 语言和 Whisper 提示词，这些设置对于本地和在线的 Whisper 服务都有效。

通常情况下，`auto` 语言选项能够自动识别语音语言，并生成相应语言的字幕。如果需要生成其他语言的字幕，或是生成的字幕语言不匹配，可以手动配置指定的语言。
根据 OpenAI 官方文档中对于 `language` 参数的描述，目前支持的语言包括

Afrikaans, Arabic, Armenian, Azerbaijani, Belarusian, Bosnian, Bulgarian,
Catalan, Chinese, Croatian, Czech, Danish, Dutch, English, Estonian, Finnish,
French, Galician, German, Greek, Hebrew, Hindi, Hungarian, Icelandic,
Indonesian, Italian, Japanese, Kannada, Kazakh, Korean, Latvian, Lithuanian,
Macedonian, Malay, Marathi, Maori, Nepali, Norwegian, Persian, Polish,
Portuguese, Romanian, Russian, Serbian, Slovak, Slovenian, Spanish, Swahili,
Swedish, Tagalog, Tamil, Thai, Turkish, Ukrainian, Urdu, Vietnamese, and Welsh.

提示词可以优化生成的字幕的风格（也会一定程度上影响质量），要注意，Whisper 无法理解复杂的提示词，你可以在提示词中使用一些简单的描述，让其在选择词汇时使用偏向于提示词所描述的领域相关的词汇，以避免出现毫不相干领域的词汇；或是让它在标点符号的使用上参照提示词的风格。
