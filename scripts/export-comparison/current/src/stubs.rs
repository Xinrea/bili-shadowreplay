//! App-owned data and a silent reporter only. The render, frame ownership,
//! scheduling, encoder selection and media implementation use production files.

pub mod project {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub enum VideoCodec {
        #[default]
        Hevc,
        H264,
    }
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub enum EncodingPriority {
        #[default]
        Speed,
        Quality,
    }
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RenderSettings {
        pub codec: VideoCodec,
        pub priority: EncodingPriority,
        pub bitrate_kbps: u32,
    }
    impl Default for RenderSettings {
        fn default() -> Self {
            Self {
                codec: VideoCodec::Hevc,
                priority: EncodingPriority::Speed,
                bitrate_kbps: 10_000,
            }
        }
    }
}

pub mod danmu2ass {
    use std::path::PathBuf;
    pub const PLAY_RES_Y: f64 = 720.0;
    pub struct DanmuImage {
        pub path: PathBuf,
        pub width: f64,
        pub height: f64,
    }
    pub struct ImageDanmu {
        pub image: DanmuImage,
        pub start: f64,
        pub end: f64,
        pub from: f64,
        pub to: f64,
        pub top: f64,
    }
}

pub mod progress {
    pub mod progress_reporter {
        #[derive(Clone, Debug)]
        pub struct StageProgress {
            pub stage: String,
            pub processed: f64,
            pub total: Option<f64>,
            pub speed: Option<f64>,
        }
        #[async_trait::async_trait]
        pub trait ProgressReporterTrait: Send + Sync + Clone {
            async fn update(&self, content: &str);
            async fn progress(&self, _progress: StageProgress) {}
            async fn finish(&self, success: bool, message: &str);
            fn stage(&self, _stage: &str) -> Self {
                self.clone()
            }
        }
    }
}

#[derive(Clone)]
pub struct Silent;
#[async_trait::async_trait]
impl progress::progress_reporter::ProgressReporterTrait for Silent {
    async fn update(&self, _content: &str) {}
    async fn finish(&self, _success: bool, _message: &str) {}
}
