//! Narrow stand-ins for the production module's two app-owned dependencies.
//! They exist only to make the production runner reusable by this benchmark;
//! renderer, queueing and ffmpeg code remain the real implementations.

pub mod danmu2ass {
    use std::path::PathBuf;
    pub const PLAY_RES_Y: f64 = 720.0;
    #[derive(Clone)]
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
        use async_trait::async_trait;
        #[derive(Clone, Debug)]
        pub struct StageProgress {
            pub stage: String,
            pub processed: f64,
            pub total: Option<f64>,
            pub speed: Option<f64>,
        }
        #[async_trait]
        pub trait ProgressReporterTrait: Send + Sync + Clone {
            async fn update(&self, _content: &str);
            async fn progress(&self, _progress: StageProgress) {}
            async fn finish(&self, _success: bool, _message: &str);
            fn stage(&self, _stage: &str) -> Self
            where
                Self: Sized,
            {
                self.clone()
            }
        }
    }
}

#[derive(Clone)]
pub struct NoReporter;
#[async_trait::async_trait]
impl progress::progress_reporter::ProgressReporterTrait for NoReporter {
    async fn update(&self, _content: &str) {}
    async fn finish(&self, _success: bool, _message: &str) {}
}
