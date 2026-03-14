# 平台实现指南

## 概述

BiliBili ShadowReplay 支持多个直播平台，每个平台的实现位于 `src-tauri/crates/recorder/src/platforms/<platform>/` 目录。

## 支持的平台

- **Bilibili** (哔哩哔哩)
- **Douyin** (抖音)
- **Huya** (虎牙)
- **Kuaishou** (快手)
- **TikTok** (国际版抖音)

## 平台接口

所有平台实现必须实现 `Recorder` trait：

```rust
#[async_trait]
pub trait Recorder: Send + Sync {
    /// 获取流信息
    async fn get_stream_info(&self) -> Result<StreamInfo, RecorderError>;

    /// 开始录制
    async fn start_recording(
        &self,
        output_path: &Path,
        cancel_token: CancellationToken,
    ) -> Result<(), RecorderError>;

    /// 获取弹幕流
    async fn get_danmaku_stream(&self) -> Result<DanmakuStream, RecorderError>;

    /// 检查直播状态
    async fn is_live(&self) -> Result<bool, RecorderError>;
}

#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub title: String,
    pub streamer: String,
    pub stream_url: String,
    pub quality: StreamQuality,
    pub available_qualities: Vec<StreamQuality>,
}

#[derive(Debug, Clone)]
pub enum StreamQuality {
    High,
    Medium,
    Low,
    Custom(String),
}
```

## 实现新平台

### 1. 创建平台目录

```bash
mkdir -p src-tauri/crates/recorder/src/platforms/newplatform
cd src-tauri/crates/recorder/src/platforms/newplatform
```

### 2. 创建模块文件

```rust
// mod.rs
mod api;
mod recorder;
mod danmaku;

pub use recorder::NewPlatformRecorder;
```

### 3. 实现 API 客户端

```rust
// api.rs
use reqwest::Client;
use serde::{Deserialize, Serialize};

pub struct NewPlatformApi {
    client: Client,
    base_url: String,
}

impl NewPlatformApi {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            base_url: "https://api.newplatform.com".to_string(),
        }
    }

    /// 获取直播间信息
    pub async fn get_room_info(&self, room_id: &str) -> Result<RoomInfo, ApiError> {
        let url = format!("{}/room/{}", self.base_url, room_id);
        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(ApiError::RequestFailed(response.status()));
        }

        let room_info: RoomInfo = response.json().await?;
        Ok(room_info)
    }

    /// 获取流地址
    pub async fn get_stream_url(
        &self,
        room_id: &str,
        quality: &str,
    ) -> Result<String, ApiError> {
        let url = format!("{}/stream/{}", self.base_url, room_id);
        let response = self.client
            .get(&url)
            .query(&[("quality", quality)])
            .send()
            .await?;

        let stream_data: StreamData = response.json().await?;
        Ok(stream_data.url)
    }
}

#[derive(Debug, Deserialize)]
pub struct RoomInfo {
    pub room_id: String,
    pub title: String,
    pub streamer: String,
    pub status: i32,  // 1: 直播中, 0: 未开播
}

#[derive(Debug, Deserialize)]
struct StreamData {
    url: String,
    quality: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Request failed: {0}")]
    RequestFailed(reqwest::StatusCode),

    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("Parse error: {0}")]
    ParseError(String),
}
```

### 4. 实现录制器

```rust
// recorder.rs
use super::api::NewPlatformApi;
use crate::{Recorder, RecorderError, StreamInfo, StreamQuality};
use async_trait::async_trait;
use std::path::Path;
use tokio_util::sync::CancellationToken;

pub struct NewPlatformRecorder {
    room_id: String,
    api: NewPlatformApi,
    quality: StreamQuality,
}

impl NewPlatformRecorder {
    pub fn new(room_id: String, quality: StreamQuality) -> Self {
        Self {
            room_id,
            api: NewPlatformApi::new(),
            quality,
        }
    }

    /// 从 URL 解析房间 ID
    pub fn parse_room_id(url: &str) -> Result<String, RecorderError> {
        // 实现 URL 解析逻辑
        // 例如: https://newplatform.com/live/12345 -> 12345
        url.split('/').last()
            .ok_or(RecorderError::InvalidUrl)?
            .to_string()
            .into()
    }
}

#[async_trait]
impl Recorder for NewPlatformRecorder {
    async fn get_stream_info(&self) -> Result<StreamInfo, RecorderError> {
        let room_info = self.api.get_room_info(&self.room_id).await
            .map_err(|e| RecorderError::ApiError(e.to_string()))?;

        let quality_str = match &self.quality {
            StreamQuality::High => "high",
            StreamQuality::Medium => "medium",
            StreamQuality::Low => "low",
            StreamQuality::Custom(q) => q,
        };

        let stream_url = self.api.get_stream_url(&self.room_id, quality_str).await
            .map_err(|e| RecorderError::ApiError(e.to_string()))?;

        Ok(StreamInfo {
            title: room_info.title,
            streamer: room_info.streamer,
            stream_url,
            quality: self.quality.clone(),
            available_qualities: vec![
                StreamQuality::High,
                StreamQuality::Medium,
                StreamQuality::Low,
            ],
        })
    }

    async fn start_recording(
        &self,
        output_path: &Path,
        cancel_token: CancellationToken,
    ) -> Result<(), RecorderError> {
        // 获取流信息
        let stream_info = self.get_stream_info().await?;

        // 使用 FFmpeg 录制
        let mut ffmpeg = tokio::process::Command::new("ffmpeg")
            .args(&[
                "-i", &stream_info.stream_url,
                "-c", "copy",
                "-f", "mp4",
                output_path.to_str().unwrap(),
            ])
            .spawn()
            .map_err(|e| RecorderError::FfmpegError(e.to_string()))?;

        // 等待录制完成或取消
        tokio::select! {
            result = ffmpeg.wait() => {
                match result {
                    Ok(status) if status.success() => Ok(()),
                    Ok(status) => Err(RecorderError::FfmpegError(
                        format!("FFmpeg exited with status: {}", status)
                    )),
                    Err(e) => Err(RecorderError::FfmpegError(e.to_string())),
                }
            }
            _ = cancel_token.cancelled() => {
                // 取消录制
                ffmpeg.kill().await.ok();
                Ok(())
            }
        }
    }

    async fn get_danmaku_stream(&self) -> Result<DanmakuStream, RecorderError> {
        // 实现弹幕流获取
        // 参考 danmu_stream crate
        todo!("Implement danmaku stream")
    }

    async fn is_live(&self) -> Result<bool, RecorderError> {
        let room_info = self.api.get_room_info(&self.room_id).await
            .map_err(|e| RecorderError::ApiError(e.to_string()))?;

        Ok(room_info.status == 1)
    }
}
```

### 5. 实现弹幕支持

```rust
// danmaku.rs
use danmu_stream::{DanmakuStream, DanmakuMessage};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, WebSocketStream};

pub struct NewPlatformDanmaku {
    room_id: String,
}

impl NewPlatformDanmaku {
    pub fn new(room_id: String) -> Self {
        Self { room_id }
    }

    pub async fn connect(&self) -> Result<DanmakuStream, DanmakuError> {
        // 连接到弹幕服务器
        let ws_url = format!("wss://danmaku.newplatform.com/room/{}", self.room_id);
        let (ws_stream, _) = connect_async(&ws_url).await?;

        // 创建弹幕流
        let stream = DanmakuStream::new(ws_stream);

        Ok(stream)
    }

    pub async fn parse_message(&self, data: &[u8]) -> Result<DanmakuMessage, DanmakuError> {
        // 解析平台特定的弹幕消息格式
        // 返回标准化的 DanmakuMessage
        todo!("Implement message parsing")
    }
}
```

### 6. 注册平台

在 `src-tauri/crates/recorder/src/platforms/mod.rs` 中注册新平台：

```rust
pub mod bilibili;
pub mod douyin;
pub mod huya;
pub mod kuaishou;
pub mod tiktok;
pub mod newplatform;  // 添加新平台

use crate::{Recorder, RecorderError, StreamQuality};

pub enum Platform {
    Bilibili,
    Douyin,
    Huya,
    Kuaishou,
    TikTok,
    NewPlatform,  // 添加枚举值
}

impl Platform {
    pub fn from_str(s: &str) -> Result<Self, RecorderError> {
        match s.to_lowercase().as_str() {
            "bilibili" => Ok(Platform::Bilibili),
            "douyin" => Ok(Platform::Douyin),
            "huya" => Ok(Platform::Huya),
            "kuaishou" => Ok(Platform::Kuaishou),
            "tiktok" => Ok(Platform::TikTok),
            "newplatform" => Ok(Platform::NewPlatform),  // 添加匹配
            _ => Err(RecorderError::UnsupportedPlatform(s.to_string())),
        }
    }

    pub fn create_recorder(
        &self,
        room_id: String,
        quality: StreamQuality,
    ) -> Box<dyn Recorder> {
        match self {
            Platform::Bilibili => Box::new(bilibili::BilibiliRecorder::new(room_id, quality)),
            Platform::Douyin => Box::new(douyin::DouyinRecorder::new(room_id, quality)),
            Platform::Huya => Box::new(huya::HuyaRecorder::new(room_id, quality)),
            Platform::Kuaishou => Box::new(kuaishou::KuaishouRecorder::new(room_id, quality)),
            Platform::TikTok => Box::new(tiktok::TikTokRecorder::new(room_id, quality)),
            Platform::NewPlatform => Box::new(newplatform::NewPlatformRecorder::new(room_id, quality)),  // 添加创建逻辑
        }
    }
}
```

## 平台特定注意事项

### Bilibili

- 使用 HTTP API 获取流信息
- 支持多种清晰度选择
- 弹幕通过 WebSocket 连接
- 需要处理 Cookie 认证（投稿功能）

### Douyin

- 需要解析动态生成的流地址
- 使用 WebSocket 接收弹幕
- 流地址有时效性，需要定期刷新

### Huya

- 使用 M3U8 格式的流
- 需要解析复杂的流地址格式
- 弹幕协议为自定义二进制格式

### Kuaishou

- API 需要特定的请求头
- 流地址加密，需要解密
- 弹幕格式为 JSON

### TikTok

- 需要处理地区限制
- 使用 HLS 流
- 弹幕通过 WebSocket 推送

## 测试

为每个平台创建测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_room_id() {
        let url = "https://newplatform.com/live/12345";
        let room_id = NewPlatformRecorder::parse_room_id(url).unwrap();
        assert_eq!(room_id, "12345");
    }

    #[tokio::test]
    async fn test_get_stream_info() {
        let recorder = NewPlatformRecorder::new(
            "12345".to_string(),
            StreamQuality::High,
        );

        let stream_info = recorder.get_stream_info().await.unwrap();
        assert!(!stream_info.stream_url.is_empty());
    }

    #[tokio::test]
    async fn test_is_live() {
        let recorder = NewPlatformRecorder::new(
            "12345".to_string(),
            StreamQuality::High,
        );

        let is_live = recorder.is_live().await.unwrap();
        // 根据实际情况断言
    }
}
```

## 调试技巧

1. **抓包分析**: 使用 Wireshark 或浏览器开发者工具分析平台 API
2. **日志记录**: 添加详细的日志输出
3. **错误处理**: 捕获并记录所有错误
4. **模拟测试**: 使用 mock 数据进行测试

## 最佳实践

1. **错误处理**: 正确处理网络错误、API 错误等
2. **重试机制**: 对临时性错误实现重试
3. **超时控制**: 为所有网络请求设置超时
4. **资源清理**: 确保连接和资源正确释放
5. **文档完善**: 为平台特定的实现添加注释
6. **遵循规范**: 遵守平台的使用条款和 API 限制