//! Bridge native media progress to asynchronous application reporters.
//!
//! Blocking workers own native contexts and output publication. This bridge
//! coalesces progress updates without blocking those workers on UI I/O.

use std::path::PathBuf;

use crate::progress::progress_reporter::{ProgressReporterTrait, StageProgress};

/// Run a native media job while keeping asynchronous reporters off its worker.
/// A watch channel retains only the newest update when rendering outruns UI I/O.
pub async fn run_native_media<R: ProgressReporterTrait>(
    plan: ffmpeg_utils::MediaPlan,
    mode: ProgressMode,
    context: impl Into<String>,
    reporter: Option<&R>,
) -> Result<ffmpeg_utils::MediaJobResult, String> {
    run_native_media_with_sources(plan, Default::default(), mode, context, reporter).await
}

/// Bind only the first streams explicitly referenced by our graph builders.
/// Generated silence has no input audio label and therefore requires no track.
pub(super) fn first_stream_graph_inputs(spec: &str, count: usize) -> Vec<ffmpeg_utils::GraphInput> {
    let mut inputs = Vec::new();
    for input in 0..count {
        for (kind, selector) in [
            ("v", ffmpeg_utils::StreamSelector::FirstVideo),
            ("a", ffmpeg_utils::StreamSelector::FirstAudio),
        ] {
            let label = format!("{input}:{kind}");
            if spec.contains(&format!("[{label}]")) {
                inputs.push(ffmpeg_utils::GraphInput {
                    input,
                    selector,
                    label,
                    required: true,
                });
            }
        }
    }
    inputs
}

/// Execute the same reporter bridge with bounded live frame sources.
pub async fn run_native_media_with_sources<R: ProgressReporterTrait>(
    mut plan: ffmpeg_utils::MediaPlan,
    bindings: ffmpeg_utils::MediaBindings,
    mode: ProgressMode,
    context: impl Into<String>,
    reporter: Option<&R>,
) -> Result<ffmpeg_utils::MediaJobResult, String> {
    if let ProgressMode::Stage { total, .. } = &mode {
        plan.progress_total = match total {
            ProgressTotal::FirstInput => ffmpeg_utils::MediaProgressTotal::FirstInput,
            ProgressTotal::AllInputs => ffmpeg_utils::MediaProgressTotal::AllInputs,
            ProgressTotal::Known(seconds) => ffmpeg_utils::MediaProgressTotal::Known(*seconds),
        };
    }
    let context = context.into();
    let Some(reporter) = reporter.filter(|_| !matches!(mode, ProgressMode::None)) else {
        return ffmpeg_utils::execute_with_sources(plan, bindings, None)
            .await
            .map_err(|error| format!("{context}: {error}"));
    };
    let (sender, mut receiver) = tokio::sync::watch::channel(None::<ffmpeg_utils::MediaProgress>);
    let job = ffmpeg_utils::execute_with_sources(
        plan,
        bindings,
        Some(Box::new(move |progress| {
            sender.send_replace(Some(progress));
        })),
    );
    tokio::pin!(job);
    let mut latest = None;
    let mut closed = false;
    loop {
        tokio::select! {
            result = &mut job => {
                let result = result.map_err(|error| format!("{context}: {error}"))?;
                let progress = receiver.borrow_and_update().clone().or(latest).unwrap_or(ffmpeg_utils::MediaProgress {
                    processed: result.processed,
                    total: None,
                    speed: None,
                    stage: ffmpeg_utils::MediaStage::Completed,
                });
                report_native_progress(reporter, &mode, progress, true).await;
                return Ok(result);
            }
            changed = receiver.changed(), if !closed => {
                if changed.is_err() { closed = true; continue; }
                let Some(progress) = receiver.borrow_and_update().clone() else { continue; };
                latest = Some(progress.clone());
                if progress.stage != ffmpeg_utils::MediaStage::Completed {
                    report_native_progress(reporter, &mode, progress, false).await;
                }
            }
        }
    }
}

async fn report_native_progress<R: ProgressReporterTrait>(
    reporter: &R,
    mode: &ProgressMode,
    progress: ffmpeg_utils::MediaProgress,
    completed: bool,
) {
    match mode {
        ProgressMode::None => (),
        ProgressMode::Stage { name, total } => {
            let total = match total {
                ProgressTotal::Known(total) => Some(*total),
                ProgressTotal::FirstInput | ProgressTotal::AllInputs => progress.total,
            }
            .filter(|total| total.is_finite() && *total > 0.0);
            let processed = if completed {
                total.unwrap_or(progress.processed.max(0.0))
            } else if let Some(total) = total {
                // Publishing happens after trailer/AVIO close. Reserve 100%
                // until execute_media_plan has also published the output.
                progress.processed.max(0.0).min(total * 0.999_999)
            } else {
                progress.processed.max(0.0)
            };
            reporter
                .progress(StageProgress {
                    stage: name.clone(),
                    processed,
                    total,
                    speed: progress
                        .speed
                        .filter(|speed| speed.is_finite() && *speed > 0.0),
                })
                .await;
        }
    }
}

/// How a native media job reports progress to the application.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ProgressMode {
    /// Execute without sending reporter updates.
    #[default]
    None,
    /// Report [`StageProgress`] under a user-facing stage name such as "压制弹幕".
    Stage { name: String, total: ProgressTotal },
}

/// Where a stage's total media duration comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProgressTotal {
    /// The first input's duration: single-video jobs, and overlays whose extra
    /// inputs are still images.
    FirstInput,
    /// The sum of all input durations, as in hard-cut concatenation.
    AllInputs,
    /// A duration the caller already knows, such as a trim length.
    Known(f64),
}

impl ProgressMode {
    /// Report measurable progress under `name`.
    pub fn stage(name: impl Into<String>, total: ProgressTotal) -> Self {
        Self::Stage {
            name: name.into(),
            total,
        }
    }
}

/// A best-effort RAII guard for a temporary media artifact.
///
/// `Drop` cannot await, so cleanup intentionally uses the synchronous standard
/// library call. This is only for artifacts that are deleted as soon as the job
/// that produced them is done; anything the caller keeps must not be guarded.
#[derive(Debug)]
pub struct TempArtifact {
    path: PathBuf,
    keep: bool,
}

impl TempArtifact {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            keep: false,
        }
    }

    /// Preserve a completed output; failed or cancelled jobs keep the guard armed.
    pub fn keep(mut self) {
        self.keep = true;
    }
}

impl Drop for TempArtifact {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod native_media_tests {
    use super::*;
    use async_trait::async_trait;
    use ffmpeg_utils::{
        MediaInput, MediaOutput, MediaPlan, StreamMode, StreamRoute, StreamSelector,
    };
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct Reporter {
        output: PathBuf,
        progress: Arc<Mutex<Vec<StageProgress>>>,
    }
    #[async_trait]
    impl ProgressReporterTrait for Reporter {
        async fn update(&self, _: &str) {}
        async fn progress(&self, progress: StageProgress) {
            if progress.percent() == Some(100) {
                assert!(self.output.is_file());
                assert_ne!(std::fs::read(&self.output).unwrap(), b"previous video");
            }
            self.progress.lock().unwrap().push(progress);
        }
        async fn finish(&self, _: bool, _: &str) {}
    }

    fn plan(input: PathBuf, output: PathBuf) -> MediaPlan {
        MediaPlan {
            inputs: vec![MediaInput::file(input)],
            outputs: vec![MediaOutput::file(output)],
            routes: vec![StreamRoute {
                source: ffmpeg_utils::StreamSource::Input {
                    input: 0,
                    selector: StreamSelector::AutoVideo,
                },
                output: 0,
                required: true,
                mode: StreamMode::Copy,
            }],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn native_progress_finishes_only_after_publication_without_cli() {
        let directory = tempfile::tempdir().unwrap();
        let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/video/test.mp4");
        let output = directory.path().join("output.mp4");
        let reporter = Reporter {
            output: output.clone(),
            progress: Default::default(),
        };
        run_native_media(
            plan(input, output),
            ProgressMode::stage("原生转封装", ProgressTotal::Known(5.45)),
            "Native test",
            Some(&reporter),
        )
        .await
        .unwrap();
        let progress = reporter.progress.lock().unwrap();
        assert!(!progress.is_empty());
        assert_eq!(progress.last().unwrap().percent(), Some(100));
        assert!(progress[..progress.len() - 1]
            .iter()
            .all(|progress| progress.percent().is_none_or(|percent| percent < 100)));
        assert!(progress
            .iter()
            .all(|progress| progress.stage == "原生转封装"));
    }

    #[tokio::test]
    async fn failed_native_progress_preserves_output_and_does_not_claim_completion() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("missing.mp4");
        let output = directory.path().join("kept.mp4");
        std::fs::write(&output, b"previous video").unwrap();
        let reporter = Reporter {
            output: output.clone(),
            progress: Default::default(),
        };
        let error = run_native_media(
            plan(input, output.clone()),
            ProgressMode::stage("原生转封装", ProgressTotal::Known(1.0)),
            "Native test",
            Some(&reporter),
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("Native test: "));
        assert!(reporter
            .progress
            .lock()
            .unwrap()
            .iter()
            .all(|progress| progress.percent() != Some(100)));
        assert_eq!(std::fs::read(&output).unwrap(), b"previous video");
    }

    #[test]
    fn partial_outputs_are_removed_but_completed_outputs_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let partial = dir.path().join("partial.mp4");
        std::fs::write(&partial, b"partial").unwrap();
        drop(TempArtifact::new(&partial));
        assert!(!partial.exists());
        let completed = dir.path().join("completed.mp4");
        std::fs::write(&completed, b"complete").unwrap();
        TempArtifact::new(&completed).keep();
        assert!(completed.exists());
    }
}
