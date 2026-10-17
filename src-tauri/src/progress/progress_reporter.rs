use async_trait::async_trait;
use serde::Serialize;
use std::sync::Arc;

use crate::events::{AppEvent, EventBus};

use crate::database::Database;

/// Measurable progress of one processing stage, such as an ffmpeg job.
///
/// Times are seconds of media, which is what ffmpeg reports, not wall-clock time.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StageProgress {
    /// User-facing stage name, e.g. "压制弹幕".
    pub stage: String,
    /// Media seconds processed so far.
    pub processed: f64,
    /// Media seconds the stage produces, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    /// Processing speed relative to realtime, when reported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
}

impl StageProgress {
    /// Whole completed percentage, when the total is known.
    pub fn percent(&self) -> Option<u32> {
        self.total
            .filter(|total| *total > 0.0)
            .map(|total| ((self.processed / total).clamp(0.0, 1.0) * 100.0).floor() as u32)
    }

    /// Plain-text form for consumers that only show a message, such as the task list.
    pub fn message(&self) -> String {
        match self.percent() {
            Some(percent) => format!("{} {percent}%", self.stage),
            None => format!("{} {}", self.stage, format_clock(self.processed)),
        }
    }
}

fn format_clock(seconds: f64) -> String {
    let total = seconds.max(0.0).floor() as u64;
    let (hours, minutes, secs) = (total / 3600, total % 3600 / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes:02}:{secs:02}")
    }
}

#[derive(Clone)]
pub struct ProgressReporter {
    events: EventBus,
    pub event_id: String,
    db: Arc<Database>,
    stage: Option<String>,
}

// async_trait generates #[must_use] on methods returning an already must-use Future.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait ProgressReporterTrait: Send + Sync + Clone {
    async fn update(&self, content: &str);

    /// Report measurable progress. Reporters that only handle text receive its message.
    async fn progress(&self, progress: StageProgress) {
        self.update(&progress.message()).await;
    }

    async fn finish(&self, success: bool, message: &str);

    /// Return a reporter that attributes measurable progress to `stage`.
    ///
    /// Pipelines use this to name steps in user terms ("提取片段 2/3") rather
    /// than after whichever ffmpeg job runs inside them ("裁剪").
    fn stage(&self, _stage: &str) -> Self {
        self.clone()
    }
}

impl ProgressReporter {
    pub async fn new(db: Arc<Database>, events: &EventBus, event_id: &str) -> Result<Self, String> {
        Ok(Self {
            db,
            events: events.clone(),
            event_id: event_id.to_string(),
            stage: None,
        })
    }

    async fn publish(&self, content: String, progress: Option<StageProgress>) {
        self.events.publish(AppEvent::ProgressUpdate {
            id: self.event_id.clone(),
            content: content.clone(),
            progress,
        });
        let _ = self
            .db
            .update_task(&self.event_id, "processing", &content, None)
            .await;
    }
}

#[async_trait]
impl ProgressReporterTrait for ProgressReporter {
    async fn update(&self, content: &str) {
        self.publish(content.to_string(), None).await;
    }

    async fn progress(&self, mut progress: StageProgress) {
        if let Some(stage) = &self.stage {
            progress.stage.clone_from(stage);
        }
        self.publish(progress.message(), Some(progress)).await;
    }

    async fn finish(&self, success: bool, message: &str) {
        self.events.publish(AppEvent::ProgressFinished {
            id: self.event_id.clone(),
            success,
            message: message.to_string(),
        });
    }

    fn stage(&self, stage: &str) -> Self {
        Self {
            stage: Some(stage.to_string()),
            ..self.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(processed: f64, total: Option<f64>) -> StageProgress {
        StageProgress {
            stage: "压制弹幕".into(),
            processed,
            total,
            speed: None,
        }
    }

    #[test]
    fn message_shows_percent_when_total_is_known() {
        assert_eq!(progress(12.0, Some(30.0)).message(), "压制弹幕 40%");
        // Never claim completion before ffmpeg actually finishes.
        assert_eq!(progress(29.9, Some(30.0)).message(), "压制弹幕 99%");
        assert_eq!(progress(31.0, Some(30.0)).percent(), Some(100));
    }

    #[test]
    fn message_falls_back_to_processed_time() {
        assert_eq!(progress(75.4, None).message(), "压制弹幕 01:15");
        assert_eq!(progress(3725.0, Some(0.0)).message(), "压制弹幕 1:02:05");
    }
}
