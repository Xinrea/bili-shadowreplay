//! Reusable ffmpeg process runner.
//!
//! The runner owns the process plumbing shared by ffmpeg jobs: it uses the
//! project's ffmpeg command factory, reads stderr through the sidecar parser,
//! reports selected progress events, and treats every non-success exit status
//! as an error.  Callers only need to provide ffmpeg arguments and, when
//! desired, a [`ProgressReporterTrait`] implementation.

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    io,
    path::PathBuf,
    process::ExitStatus,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_ffmpeg_sidecar::{
    event::FfmpegEvent,
    log_parser::{parse_time_str, FfmpegLogParser},
};
use async_trait::async_trait;
use thiserror::Error;
use tokio::{
    io::{AsyncWriteExt, BufReader},
    process::{ChildStdin, Command},
    sync::mpsc,
};

use crate::progress::progress_reporter::{ProgressReporterTrait, StageProgress};
use ffmpeg_utils::ffmpeg_command;

/// How a job turns parsed ffmpeg events into progress updates.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ProgressMode {
    /// Do not call the reporter and do not add ffmpeg's `-progress` option.
    #[default]
    None,
    /// Report the human-readable timestamp from `FfmpegEvent::Progress`.
    ///
    /// The parser also recognizes the `out_time=` key emitted by
    /// `-progress pipe:2`, so that value is used when no summary progress event
    /// is available.
    Time,
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

    fn needs_progress_pipe(&self) -> bool {
        !matches!(self, Self::None)
    }

    fn progress_time(event: &FfmpegEvent) -> Option<&str> {
        match event {
            FfmpegEvent::Progress(progress) => Some(progress.time.as_str()),
            FfmpegEvent::Log(_, content) => content
                .strip_prefix("out_time=")
                .map(str::trim)
                .filter(|content| !content.is_empty()),
            _ => None,
        }
    }
}

enum ProgressReport {
    Text(String),
    Stage(StageProgress),
}

/// Follows one job's ffmpeg events and turns them into progress reports.
#[derive(Default)]
struct ProgressTracker {
    input_durations: BTreeMap<u32, f64>,
    speed: Option<f64>,
    last_processed: Option<f64>,
}

impl ProgressTracker {
    fn observe(&mut self, mode: &ProgressMode, event: &FfmpegEvent) -> Option<ProgressReport> {
        let (name, total) = match mode {
            ProgressMode::None => return None,
            ProgressMode::Time => {
                return ProgressMode::progress_time(event)
                    .map(|time| ProgressReport::Text(time.to_owned()));
            }
            ProgressMode::Stage { name, total } => (name, *total),
        };

        match event {
            FfmpegEvent::ParsedDuration(duration) => {
                self.input_durations
                    .insert(duration.input_index, duration.duration);
            }
            FfmpegEvent::Progress(progress) if progress.speed > 0.0 => {
                self.speed = Some(f64::from(progress.speed));
            }
            FfmpegEvent::Log(_, content) => {
                if let Some(speed) = content
                    .strip_prefix("speed=")
                    .and_then(|speed| speed.trim().trim_end_matches('x').parse::<f64>().ok())
                    .filter(|speed| *speed > 0.0)
                {
                    self.speed = Some(speed);
                }
            }
            _ => {}
        }

        // Negative before the first output frame, e.g. "-577014:32:22.77".
        let processed = ProgressMode::progress_time(event)
            .and_then(parse_time_str)?
            .max(0.0);
        // The stats line and the -progress block repeat the same position.
        if self.last_processed == Some(processed) {
            return None;
        }
        self.last_processed = Some(processed);

        Some(ProgressReport::Stage(StageProgress {
            stage: name.clone(),
            processed,
            total: self.total(total),
            speed: self.speed,
        }))
    }

    fn total(&self, total: ProgressTotal) -> Option<f64> {
        match total {
            ProgressTotal::FirstInput => self.input_durations.get(&0).copied(),
            ProgressTotal::AllInputs => {
                (!self.input_durations.is_empty()).then(|| self.input_durations.values().sum())
            }
            ProgressTotal::Known(total) => Some(total),
        }
        .filter(|total| *total > 0.0)
    }
}

/// A configured ffmpeg invocation.
///
/// `args` contains arguments supplied before the optional output path.  When
/// [`FfmpegJob::output`] is set, that path is appended as the final command
/// argument and is also returned in [`JobOutput`].  This keeps the common
/// "arguments plus one output file" case from being repeated at every call
/// site.  If a caller needs a different ffmpeg argument layout, it can leave
/// `output` unset and put every argument in `args` directly.
#[derive(Debug, Clone, PartialEq)]
pub struct FfmpegJob {
    pub args: Vec<OsString>,
    pub output: Option<PathBuf>,
    pub progress: ProgressMode,
    /// Optional label included in all runner errors.
    pub context: Option<String>,
}

impl Default for FfmpegJob {
    fn default() -> Self {
        Self {
            args: Vec::new(),
            output: None,
            progress: ProgressMode::None,
            context: None,
        }
    }
}

impl FfmpegJob {
    /// Create an empty job.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a job with an initial argument list.
    pub fn with_args<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        Self::new().args(args)
    }

    /// Add arguments to this job.
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Set the output path appended to the ffmpeg command.
    pub fn output<P>(mut self, output: P) -> Self
    where
        P: Into<PathBuf>,
    {
        self.output = Some(output.into());
        self
    }

    /// Set how parsed ffmpeg progress is reported.
    pub fn progress(mut self, progress: ProgressMode) -> Self {
        self.progress = progress;
        self
    }

    /// Set the label used to identify this job in errors.
    pub fn context<S>(mut self, context: S) -> Self
    where
        S: Into<String>,
    {
        self.context = Some(context.into());
        self
    }

    /// Run this job using the application's ffmpeg command factory.
    ///
    /// The reporter is borrowed for the duration of the run.  It is generic
    /// rather than a trait object because [`ProgressReporterTrait`] requires
    /// `Clone` and is therefore not object-safe.
    pub async fn run<R>(&self, reporter: Option<&R>) -> Result<JobOutput, FfmpegError>
    where
        R: ProgressReporterTrait,
    {
        self.run_with_command(ffmpeg_command(), reporter).await
    }

    /// Run this job without progress reporting.
    ///
    /// This is also the convenient alternative to `run(None::<&Reporter>)`
    /// when the reporter type is not otherwise available at the call site.
    pub async fn run_without_reporter(&self) -> Result<JobOutput, FfmpegError> {
        self.run_with_command(ffmpeg_command(), None::<&NoopReporter>)
            .await
    }

    fn command_context(&self) -> String {
        self.context
            .as_deref()
            .filter(|context| !context.trim().is_empty())
            .unwrap_or("ffmpeg")
            .to_owned()
    }

    fn configure_command(&self, mut command: Command) -> Command {
        // ffmpeg_command() already sets this, but keeping the invariant here
        // protects the runner if the shared command factory ever changes and
        // also applies it to the test command injected below.
        command.kill_on_drop(true);
        command.args(&self.args);

        // `-progress pipe:2` produces the key/value events consumed by the
        // progress modes. Respect an explicitly supplied -progress argument so
        // callers can choose a different ffmpeg progress destination.
        if self.progress.needs_progress_pipe() && !has_progress_argument(&self.args) {
            command.args(["-progress", "pipe:2"]);
        }

        if let Some(output) = &self.output {
            command.arg(output);
        }

        command.stderr(Stdio::piped());
        command
    }

    async fn run_with_command<R>(
        &self,
        command: Command,
        reporter: Option<&R>,
    ) -> Result<JobOutput, FfmpegError>
    where
        R: ProgressReporterTrait,
    {
        self.run_with_frames(command, None, reporter).await
    }

    async fn run_with_frames<R: ProgressReporterTrait>(
        &self,
        command: Command,
        frames: Option<RawFrameReceiver>,
        reporter: Option<&R>,
    ) -> Result<JobOutput, FfmpegError> {
        let context = self.command_context();
        let mut command = self.configure_command(command);
        // Existing jobs retain exactly the stdin configured by their caller.
        if frames.is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = command.spawn().map_err(|source| FfmpegError::Spawn {
            context: context.clone(),
            source,
        })?;
        let stdin = if frames.is_some() {
            child.stdin.take()
        } else {
            None
        };

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| FfmpegError::StderrPipeUnavailable {
                context: context.clone(),
            })?;
        let mut parser = FfmpegLogParser::new(BufReader::new(stderr));
        let mut tracker = ProgressTracker::default();
        let mut logs = Vec::new();

        let process = async {
            loop {
                // Do not use `while let Ok(...)` here: parser/I/O failures are
                // meaningful job failures and must reach the caller.
                let event =
                    parser
                        .parse_next_event()
                        .await
                        .map_err(|error| FfmpegError::Parser {
                            context: context.clone(),
                            message: error.to_string(),
                        })?;

                match event {
                    FfmpegEvent::LogEOF => break,
                    FfmpegEvent::Error(message) => {
                        return Err(FfmpegError::Event {
                            context: context.clone(),
                            message,
                        });
                    }
                    event => {
                        if let FfmpegEvent::Log(_, content) = &event {
                            logs.push(content.clone());
                        }
                        if let Some(reporter) = reporter {
                            match tracker.observe(&self.progress, &event) {
                                Some(ProgressReport::Text(message)) => {
                                    reporter.update(&message).await
                                }
                                Some(ProgressReport::Stage(progress)) => {
                                    reporter.progress(progress).await
                                }
                                None => {}
                            }
                        }
                    }
                }
            }

            let status = child.wait().await.map_err(|source| FfmpegError::Wait {
                context: context.clone(),
                source,
            })?;
            if !status.success() {
                return Err(FfmpegError::NonZeroExit {
                    context: context.clone(),
                    status,
                    detail: failure_detail(&logs),
                });
            }

            Ok(JobOutput {
                output: self.output.clone(),
                status,
            })
        };
        tokio::pin!(process);
        let feed = feed_raw_frames(stdin, frames);
        tokio::pin!(feed);
        tokio::select! {
            // Prefer a ready producer failure over a simultaneous process exit.
            biased;
            result = &mut feed => {
                match result {
                    Ok(()) => process.await,
                    Err(FrameFeedError::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe => {
                        // FFmpeg may stop reading when its primary video ends before
                        // the probed duration. Only its successful exit legitimizes
                        // this closure; never mask a failed encode with pipe EOF.
                        process.await
                    }
                    Err(error) => Err(FfmpegError::FrameFeed {
                        context: context.clone(), message: error.to_string(),
                    }),
                }
            }
            result = &mut process => result,
        }
        // Both futures live in this scope, not detached tasks: cancellation
        // drops stdin + the receiver and kills the child (kill_on_drop).
    }
}

/// The last lines ffmpeg wrote before failing, as "：line / line", or "".
/// `-progress` key/value lines and the stats line say nothing about the cause.
fn failure_detail(logs: &[String]) -> String {
    let is_progress = |line: &str| {
        line.split_once('=').is_some_and(|(key, _)| {
            !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }) || line.starts_with("frame=")
            || line.starts_with("size=")
    };
    let mut lines: Vec<&str> = logs
        .iter()
        .rev()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !is_progress(line))
        .take(2)
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    lines.reverse();
    let detail: String = lines.join(" / ").chars().take(300).collect();
    format!("：{detail}")
}

/// Run a command that has already been assembled by a feature-specific module.
///
/// This keeps hardware-acceleration builders, which currently operate on a
/// `tokio::process::Command`, on the same stderr parser and exit-status path as
/// [`FfmpegJob`].
pub async fn run_command<R>(
    command: Command,
    progress: ProgressMode,
    context: impl Into<String>,
    reporter: Option<&R>,
) -> Result<JobOutput, FfmpegError>
where
    R: ProgressReporterTrait,
{
    FfmpegJob::new()
        .progress(progress)
        .context(context)
        .run_with_command(command, reporter)
        .await
}

/// A bounded raw-video stream must explicitly finish. Dropping its producer
/// (including a panic) is not a successful EOF.
pub enum RawFrame {
    Frame(Vec<u8>),
    Finished,
}

/// Feed-stage wall times. Pipe writes include OS/FFmpeg backpressure, not just
/// copying and not pure encoding time. Queue wait overlaps producer rendering.
#[derive(Clone, Debug, Default)]
pub struct FrameFeedStats {
    pub frames: u64,
    pub bytes: u64,
    pub queue_wait: Duration,
    pub pipe_write: Duration,
    pub(crate) feed_finished_at: Option<Instant>,
}

pub struct RawFrameReceiver {
    pub receiver: mpsc::Receiver<Result<RawFrame, String>>,
    pub frame_bytes: usize,
    pub frame_count: u64,
    /// Return ownership only after write_all has consumed the entire frame.
    /// Dropping this sender must wake a producer waiting for a reusable buffer.
    pub recycle: Option<mpsc::Sender<Vec<u8>>>,
    pub stats: Option<Arc<Mutex<FrameFeedStats>>>,
}

struct FeedProfile {
    destination: Option<Arc<Mutex<FrameFeedStats>>>,
    value: FrameFeedStats,
}

impl FeedProfile {
    fn start(&self) -> Option<Instant> {
        self.destination.as_ref().map(|_| Instant::now())
    }
}

impl Drop for FeedProfile {
    fn drop(&mut self) {
        if let Some(destination) = &self.destination {
            *destination.lock().unwrap_or_else(|e| e.into_inner()) = self.value.clone();
        }
    }
}

#[derive(Debug, Error)]
enum FrameFeedError {
    #[error("{0}")]
    Stream(String),
    #[error("failed writing raw video: {0}")]
    Io(#[from] io::Error),
}

async fn feed_raw_frames(
    stdin: Option<ChildStdin>,
    frames: Option<RawFrameReceiver>,
) -> Result<(), FrameFeedError> {
    let Some(mut frames) = frames else {
        return std::future::pending().await;
    };
    let mut stdin = stdin.ok_or_else(|| FrameFeedError::Stream("stdin pipe unavailable".into()))?;
    let mut written = 0;
    let mut profile = FeedProfile {
        destination: frames.stats.take(),
        value: FrameFeedStats::default(),
    };
    loop {
        let started = profile.start();
        let received = frames.receiver.recv().await;
        if let Some(started) = started {
            profile.value.queue_wait += started.elapsed();
        }
        let message = received
            .ok_or_else(|| {
                FrameFeedError::Stream("raw-frame producer closed without finishing".into())
            })?
            .map_err(FrameFeedError::Stream)?;
        match message {
            RawFrame::Frame(bytes) => {
                if bytes.len() != frames.frame_bytes || written >= frames.frame_count {
                    return Err(FrameFeedError::Stream(
                        "invalid raw-frame size or count".into(),
                    ));
                }
                let started = profile.start();
                let result = stdin.write_all(&bytes).await;
                if let Some(started) = started {
                    profile.value.pipe_write += started.elapsed();
                }
                result?;
                written += 1;
                profile.value.frames += 1;
                profile.value.bytes += bytes.len() as u64;
                if let Some(recycle) = &frames.recycle {
                    // Closed means the producer has already finished or failed.
                    // Never await here: the last frames may have no consumer.
                    let _ = recycle.try_send(bytes);
                }
            }
            RawFrame::Finished => {
                if written != frames.frame_count {
                    return Err(FrameFeedError::Stream(format!(
                        "raw-frame stream ended after {written} of {} frames",
                        frames.frame_count
                    )));
                }
                stdin.shutdown().await?;
                if profile.destination.is_some() {
                    profile.value.feed_finished_at = Some(Instant::now());
                }
                return Ok(());
            }
        }
    }
}

/// Run the same job pipeline with an explicitly opted-in raw-video stdin.
/// The receiver is owned here so cancellation always releases a blocked sender.
pub async fn run_command_with_frames<R: ProgressReporterTrait>(
    command: Command,
    frames: RawFrameReceiver,
    progress: ProgressMode,
    context: impl Into<String>,
    reporter: Option<&R>,
) -> Result<JobOutput, FfmpegError> {
    FfmpegJob::new()
        .progress(progress)
        .context(context)
        .run_with_frames(command, Some(frames), reporter)
        .await
}

fn has_progress_argument(args: &[OsString]) -> bool {
    args.iter()
        .any(|arg| arg.as_os_str() == OsStr::new("-progress"))
}

/// Errors produced while spawning, parsing, or waiting for an ffmpeg job.
#[derive(Debug, Error)]
pub enum FfmpegError {
    #[error("{context}: failed to spawn ffmpeg: {source}")]
    Spawn {
        context: String,
        #[source]
        source: io::Error,
    },
    #[error("{context}: raw-video stream failed: {message}")]
    FrameFeed { context: String, message: String },
    #[error("{context}: ffmpeg stderr pipe unavailable")]
    StderrPipeUnavailable { context: String },
    #[error("{context}: failed to parse ffmpeg stderr: {message}")]
    Parser { context: String, message: String },
    #[error("{context}: ffmpeg reported an error: {message}")]
    Event { context: String, message: String },
    #[error("{context}: failed waiting for ffmpeg: {source}")]
    Wait {
        context: String,
        #[source]
        source: io::Error,
    },
    #[error("{context}: ffmpeg 异常退出（{status}）{detail}")]
    NonZeroExit {
        context: String,
        status: ExitStatus,
        /// The last lines ffmpeg logged, which usually name the cause.
        detail: String,
    },
}

/// The successful result of an ffmpeg job.
#[derive(Debug)]
pub struct JobOutput {
    /// The output configured on the job, if any.
    #[allow(dead_code)]
    pub output: Option<PathBuf>,
    /// The successful process status.
    #[allow(dead_code)]
    pub status: ExitStatus,
}

/// A best-effort RAII guard for a temporary file produced by ffmpeg.
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

/// A private reporter used only by `run_without_reporter` to keep the runner's
/// implementation generic without requiring a trait object.
#[derive(Clone, Copy)]
struct NoopReporter;

#[async_trait]
impl ProgressReporterTrait for NoopReporter {
    async fn update(&self, _content: &str) {}

    async fn finish(&self, _success: bool, _message: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_ffmpeg_sidecar::event::{FfmpegDuration, FfmpegProgress, LogLevel};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct RecordingReporter(Arc<Mutex<Vec<String>>>);

    #[async_trait]
    impl ProgressReporterTrait for RecordingReporter {
        async fn update(&self, content: &str) {
            self.0
                .lock()
                .expect("recording reporter mutex poisoned")
                .push(content.to_owned());
        }

        async fn finish(&self, _success: bool, _message: &str) {}
    }

    fn sample_progress_event() -> FfmpegEvent {
        FfmpegEvent::Progress(FfmpegProgress {
            frame: 1,
            fps: 1.0,
            q: 0.0,
            size_kb: 1,
            time: "00:00:01.00".to_owned(),
            bitrate_kbps: 1.0,
            speed: 1.0,
            raw_log_message: String::new(),
        })
    }

    fn observe_all(mode: &ProgressMode, events: &[FfmpegEvent]) -> Vec<ProgressReport> {
        let mut tracker = ProgressTracker::default();
        events
            .iter()
            .filter_map(|event| tracker.observe(mode, event))
            .collect()
    }

    fn duration_event(input_index: u32, duration: f64) -> FfmpegEvent {
        FfmpegEvent::ParsedDuration(FfmpegDuration {
            input_index,
            duration,
            raw_log_message: String::new(),
        })
    }

    fn stage_reports(reports: Vec<ProgressReport>) -> Vec<StageProgress> {
        reports
            .into_iter()
            .map(|report| match report {
                ProgressReport::Stage(progress) => progress,
                ProgressReport::Text(text) => panic!("expected stage progress, got {text:?}"),
            })
            .collect()
    }

    #[test]
    fn time_mode_reports_raw_timestamps() {
        let progress = sample_progress_event();
        assert!(observe_all(&ProgressMode::None, std::slice::from_ref(&progress)).is_empty());
        let out_time = FfmpegEvent::Log(LogLevel::Info, "out_time=00:00:01.00".to_owned());
        let reports = observe_all(&ProgressMode::Time, &[progress, out_time]);
        let texts: Vec<_> = reports
            .into_iter()
            .map(|report| match report {
                ProgressReport::Text(text) => text,
                ProgressReport::Stage(progress) => panic!("unexpected {progress:?}"),
            })
            .collect();
        assert_eq!(texts, ["00:00:01.00", "00:00:01.00"]);
    }

    #[test]
    fn stage_mode_measures_against_the_first_input() {
        let mode = ProgressMode::stage("压制弹幕", ProgressTotal::FirstInput);
        let reports = stage_reports(observe_all(
            &mode,
            &[
                duration_event(0, 4.0),
                // A still-image overlay input must not change the total.
                duration_event(1, 0.04),
                sample_progress_event(),
                // The -progress block repeats the stats line position.
                FfmpegEvent::Log(LogLevel::Info, "out_time=00:00:01.000000".to_owned()),
                FfmpegEvent::Log(LogLevel::Info, "speed=2.5x".to_owned()),
                FfmpegEvent::Log(LogLevel::Info, "out_time=00:00:03.000000".to_owned()),
            ],
        ));
        assert_eq!(
            reports,
            [
                StageProgress {
                    stage: "压制弹幕".into(),
                    processed: 1.0,
                    total: Some(4.0),
                    speed: Some(1.0),
                },
                StageProgress {
                    stage: "压制弹幕".into(),
                    processed: 3.0,
                    total: Some(4.0),
                    speed: Some(2.5),
                },
            ]
        );
    }

    #[test]
    fn stage_mode_totals_can_sum_inputs_or_be_known() {
        let events = [
            duration_event(0, 4.0),
            duration_event(1, 6.0),
            sample_progress_event(),
        ];
        let summed = stage_reports(observe_all(
            &ProgressMode::stage("合并视频", ProgressTotal::AllInputs),
            &events,
        ));
        assert_eq!(summed[0].total, Some(10.0));
        let known = stage_reports(observe_all(
            &ProgressMode::stage("裁剪", ProgressTotal::Known(2.0)),
            &events,
        ));
        assert_eq!(known[0].total, Some(2.0));
        let unknown = stage_reports(observe_all(
            &ProgressMode::stage("合并视频", ProgressTotal::FirstInput),
            &[sample_progress_event()],
        ));
        assert_eq!(unknown[0].total, None);
    }

    #[test]
    fn stage_mode_clamps_negative_start_times() {
        let reports = stage_reports(observe_all(
            &ProgressMode::stage("压制弹幕", ProgressTotal::FirstInput),
            &[FfmpegEvent::Log(
                LogLevel::Info,
                "out_time=-577014:32:22.771233".to_owned(),
            )],
        ));
        assert_eq!(reports[0].processed, 0.0);
    }

    fn raw_receiver(
        receiver: mpsc::Receiver<Result<RawFrame, String>>,
        frame_bytes: usize,
        frame_count: u64,
    ) -> RawFrameReceiver {
        RawFrameReceiver {
            receiver,
            frame_bytes,
            frame_count,
            recycle: None,
            stats: None,
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn raw_frames_return_the_same_owned_allocation_after_writing() {
        let (sender, receiver) = mpsc::channel(1);
        let (recycle, mut recycled) = mpsc::channel(1);
        let stats = Arc::new(Mutex::new(FrameFeedStats::default()));
        let mut input = raw_receiver(receiver, 4096, 8);
        input.recycle = Some(recycle);
        input.stats = Some(stats.clone());
        let producer = async move {
            let mut bytes = vec![0u8; 4096];
            let pointer = bytes.as_ptr() as usize;
            for i in 0..8 {
                bytes.fill(i);
                sender.send(Ok(RawFrame::Frame(bytes))).await.unwrap();
                bytes = recycled.recv().await.unwrap();
                assert_eq!(
                    bytes.as_ptr() as usize,
                    pointer,
                    "writer must return ownership, not clone"
                );
                assert!(bytes.iter().all(|b| *b == i));
            }
            sender.send(Ok(RawFrame::Finished)).await.unwrap();
        };
        let mut command = Command::new("sh");
        command.args(["-c", "cat >/dev/null"]);
        let run = FfmpegJob::new();
        let (_, output) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(
                producer,
                run.run_with_frames(command, Some(input), None::<&RecordingReporter>)
            )
        })
        .await
        .unwrap();
        output.unwrap();
        let stats = stats.lock().unwrap();
        assert_eq!((stats.frames, stats.bytes), (8, 8 * 4096));
        assert!(stats.feed_finished_at.is_some());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn raw_stream_requires_explicit_complete_eof_and_valid_frames() {
        for (messages, expected) in [
            (
                vec![Ok(RawFrame::Frame(vec![1; 4])), Ok(RawFrame::Finished)],
                None,
            ),
            (
                vec![Ok(RawFrame::Frame(vec![1; 4]))],
                Some("without finishing"),
            ),
            (vec![Err("render failed".into())], Some("render failed")),
            (
                vec![Ok(RawFrame::Frame(vec![1; 3]))],
                Some("invalid raw-frame"),
            ),
            (vec![Ok(RawFrame::Finished)], Some("0 of 1")),
        ] {
            let (sender, receiver) = mpsc::channel(2);
            for message in messages {
                sender.send(message).await.unwrap();
            }
            drop(sender);
            let result = run_command_with_frames(
                test_command("cat >/dev/null"),
                raw_receiver(receiver, 4, 1),
                ProgressMode::None,
                "stream test",
                None::<&RecordingReporter>,
            )
            .await;
            match expected {
                None => assert!(result.is_ok(), "{result:?}"),
                Some(expected) => assert!(result.unwrap_err().to_string().contains(expected)),
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn raw_stdin_closure_never_masks_a_failed_process() {
        let (sender, receiver) = mpsc::channel(1);
        let producer = tokio::spawn(async move {
            loop {
                if sender
                    .send(Ok(RawFrame::Frame(vec![0; 1024 * 1024])))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let result = run_command_with_frames(
            test_command("exit 17"),
            raw_receiver(receiver, 1024 * 1024, u64::MAX),
            ProgressMode::None,
            "early failure",
            None::<&RecordingReporter>,
        )
        .await;
        assert!(
            matches!(result, Err(FfmpegError::NonZeroExit { .. })),
            "{result:?}"
        );
        producer.await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_raw_stream_closes_receiver_and_kills_process() {
        use tokio::time::{sleep, timeout, Duration};
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("pid");
        let mut command = Command::new("sh");
        command
            .args(["-c", "printf '%s' \"$$\" > \"$1\"; exec sleep 60", "test"])
            .arg(&pid_file);
        let (sender, receiver) = mpsc::channel(1);
        sender
            .send(Ok(RawFrame::Frame(vec![0; 1024 * 1024])))
            .await
            .unwrap();
        let job = tokio::spawn(async move {
            run_command_with_frames(
                command,
                raw_receiver(receiver, 1024 * 1024, 1),
                ProgressMode::None,
                "cancellation",
                None::<&RecordingReporter>,
            )
            .await
        });
        timeout(Duration::from_secs(3), async {
            while !pid_file.exists() {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let pid = std::fs::read_to_string(&pid_file).unwrap();
        job.abort();
        assert!(job.await.unwrap_err().is_cancelled());
        timeout(Duration::from_secs(1), sender.closed())
            .await
            .unwrap();
        timeout(Duration::from_secs(3), async {
            while Command::new("kill")
                .args(["-0", &pid])
                .stderr(Stdio::null())
                .status()
                .await
                .unwrap()
                .success()
            {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancelled raw-video child must exit");
    }

    #[tokio::test]
    #[ignore = "requires FFmpeg"]
    async fn raw_video_may_end_naturally_before_the_overlay_spec() {
        use tokio::time::{timeout, Duration};
        let (sender, receiver) = mpsc::channel(1);
        let producer = tokio::spawn(async move {
            for _ in 0..1000 {
                if sender
                    .send(Ok(RawFrame::Frame(vec![0; 64 * 64 * 4])))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            let _ = sender.send(Ok(RawFrame::Finished)).await;
        });
        let mut command = ffmpeg_command();
        command
            .args([
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "color=s=64x64:r=60:d=0.1",
                "-f",
                "rawvideo",
                "-pixel_format",
                "rgba",
                "-video_size",
                "64x64",
                "-framerate",
                "60",
                "-i",
                "pipe:0",
                "-filter_complex",
                "[0:v][1:v]overlay=shortest=1:alpha=straight[outv]",
                "-map",
                "[outv]",
                "-f",
                "null",
                "-",
            ])
            .stdout(Stdio::null());
        let result = timeout(
            Duration::from_secs(10),
            run_command_with_frames(
                command,
                raw_receiver(receiver, 64 * 64 * 4, 1000),
                ProgressMode::Time,
                "short input",
                None::<&RecordingReporter>,
            ),
        )
        .await
        .unwrap();
        assert!(result.is_ok(), "{result:?}");
        timeout(Duration::from_secs(1), producer)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn nonzero_exit_is_reported_as_structured_error() {
        let job = FfmpegJob::new().context("non-zero test");
        let result = job
            .run_with_command(test_command("exit 17"), None::<&RecordingReporter>)
            .await;

        match result {
            Err(FfmpegError::NonZeroExit {
                context, status, ..
            }) => {
                assert_eq!(context, "non-zero test");
                assert!(!status.success());
            }
            other => panic!("expected non-zero exit error, got {other:?}"),
        }
    }

    #[test]
    fn failure_detail_names_the_last_logged_problem() {
        let logs = [
            "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'a.mp4':",
            "[AVFilterGraph @ 0x1] No such filter: 'ass'",
            "Error opening output files: Filter not found",
            "out_time=00:00:00.000000",
            "progress=end",
        ]
        .map(String::from);
        assert_eq!(
            failure_detail(&logs),
            "：[AVFilterGraph @ 0x1] No such filter: 'ass' / Error opening output files: Filter not found"
        );
        assert_eq!(failure_detail(&["speed=1x".to_string()]), "");
    }

    #[tokio::test]
    async fn progress_is_forwarded_to_the_reporter() {
        let reporter = RecordingReporter::default();
        let job = FfmpegJob::new()
            .context("progress test")
            .progress(ProgressMode::Time);
        let result = job
            .run_with_command(test_command(progress_test_script()), Some(&reporter))
            .await;

        assert!(result.is_ok(), "progress test failed: {result:?}");
        assert_eq!(
            reporter
                .0
                .lock()
                .expect("recording reporter mutex poisoned")
                .as_slice(),
            [String::from("00:00:01.00")]
        );
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

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_queued_runner_terminates_real_ffmpeg_and_advances_queue() {
        use crate::task::{Task, TaskManager, TaskManagerConfig, TaskPriority, TaskStatus};
        use tokio::sync::{oneshot, Notify};
        use tokio::time::{sleep, timeout, Duration};

        #[derive(Clone)]
        struct StartedReporter(Arc<Notify>);
        #[async_trait]
        impl ProgressReporterTrait for StartedReporter {
            async fn update(&self, _: &str) {
                self.0.notify_one();
            }
            async fn finish(&self, _: bool, _: &str) {}
        }

        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("ffmpeg.pid");
        // exec keeps the PID owned by the runner. No shell child survives it.
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "printf '%s' \"$$\" > \"$1\"; shift; exec ffmpeg \"$@\"",
            "runner-test",
        ]);
        command.arg(&pid_file).args([
            "-hide_banner",
            "-re",
            "-f",
            "lavfi",
            "-i",
            "color=size=16x16:rate=10",
            "-t",
            "60",
            "-f",
            "null",
            "-",
        ]);
        let started = Arc::new(Notify::new());
        let reporter = StartedReporter(started.clone());
        let mut manager = TaskManager::with_config(TaskManagerConfig {
            max_concurrent_tasks: 1,
            check_interval_ms: 5,
            max_queue_size: 20,
        });
        manager
            .add_task(Task::new(
                "encode".into(),
                TaskPriority::Normal,
                async move {
                    FfmpegJob::new()
                        .progress(ProgressMode::Time)
                        .run_with_command(command, Some(&reporter))
                        .await
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                },
            ))
            .await
            .unwrap();
        let (next_tx, next_rx) = oneshot::channel();
        manager
            .add_task(Task::new("next".into(), TaskPriority::Normal, async move {
                let _ = next_tx.send(());
                Ok(())
            }))
            .await
            .unwrap();
        manager.start();
        timeout(Duration::from_secs(10), started.notified())
            .await
            .expect("FFmpeg must report progress");
        let pid = std::fs::read_to_string(pid_file).unwrap();
        let alive = || {
            let pid = pid.clone();
            async move {
                Command::new("kill")
                    .args(["-0", &pid])
                    .stderr(Stdio::null())
                    .status()
                    .await
                    .unwrap()
                    .success()
            }
        };
        assert!(alive().await);
        manager.cancel_task("encode").await.unwrap();
        timeout(Duration::from_secs(3), async {
            while alive().await {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancelled FFmpeg process must exit");
        timeout(Duration::from_secs(1), next_rx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            manager.get_task_status("encode").await,
            Some(TaskStatus::Cancelled)
        );
        manager.stop().await;
    }

    #[cfg(unix)]
    fn progress_test_script() -> &'static str {
        "printf '%s\\n' 'frame= 1 fps=1 q=0 size=1kB time=00:00:01.00 bitrate=1kbits/s speed=1x' >&2"
    }

    #[cfg(windows)]
    fn progress_test_script() -> &'static str {
        "echo frame= 1 fps=1 q=0 size=1kB time=00:00:01.00 bitrate=1kbits/s speed=1x 1>&2"
    }

    #[cfg(unix)]
    fn test_command(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script, "runner-test"]);
        command
    }

    #[cfg(windows)]
    fn test_command(script: &str) -> Command {
        let mut command = Command::new("cmd");
        command.args(["/C", script]);
        command
    }
}
