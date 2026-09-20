//! Reusable ffmpeg process runner.
//!
//! The runner owns the process plumbing shared by ffmpeg jobs: it uses the
//! project's ffmpeg command factory, reads stderr through the sidecar parser,
//! reports selected progress events, and treats every non-success exit status
//! as an error.  Callers only need to provide ffmpeg arguments and, when
//! desired, a [`ProgressReporterTrait`] implementation.

use std::{
    ffi::{OsStr, OsString},
    io,
    path::PathBuf,
    process::ExitStatus,
    process::Stdio,
};

use async_ffmpeg_sidecar::{event::FfmpegEvent, log_parser::FfmpegLogParser};
use async_trait::async_trait;
use thiserror::Error;
use tokio::{io::BufReader, process::Command};

use crate::progress::progress_reporter::ProgressReporterTrait;
use ffmpeg_utils::ffmpeg_command;

/// How a job turns parsed ffmpeg events into progress updates.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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
    /// Report `<prefix><timestamp><suffix>`, which keeps each job's existing
    /// user-visible wording ("压制中：…", "切片进度: …").
    Labeled { prefix: String, suffix: String },
    /// Report the raw microsecond value from an `out_time_ms=` log event.
    OutTimeMs,
}

impl ProgressMode {
    /// Report `<prefix><timestamp>`.
    pub fn prefixed(prefix: impl Into<String>) -> Self {
        Self::Labeled {
            prefix: prefix.into(),
            suffix: String::new(),
        }
    }

    /// Report `<prefix><timestamp><suffix>`.
    pub fn labeled(prefix: impl Into<String>, suffix: impl Into<String>) -> Self {
        Self::Labeled {
            prefix: prefix.into(),
            suffix: suffix.into(),
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

    fn message_for(&self, event: &FfmpegEvent) -> Option<String> {
        match self {
            Self::None => None,
            Self::Time => Self::progress_time(event).map(ToOwned::to_owned),
            Self::Labeled { prefix, suffix } => {
                Self::progress_time(event).map(|time| format!("{prefix}{time}{suffix}"))
            }
            Self::OutTimeMs => match event {
                FfmpegEvent::Log(_, content) => content
                    .strip_prefix("out_time_ms=")
                    .map(str::trim)
                    .filter(|content| !content.is_empty())
                    .map(ToOwned::to_owned),
                _ => None,
            },
        }
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
        let context = self.command_context();
        let mut child =
            self.configure_command(command)
                .spawn()
                .map_err(|source| FfmpegError::Spawn {
                    context: context.clone(),
                    source,
                })?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| FfmpegError::StderrPipeUnavailable {
                context: context.clone(),
            })?;
        let mut parser = FfmpegLogParser::new(BufReader::new(stderr));
        let mut logs = Vec::new();

        loop {
            // Do not use `while let Ok(...)` here: parser/I/O failures are
            // meaningful job failures and must reach the caller.
            let event = parser
                .parse_next_event()
                .await
                .map_err(|error| FfmpegError::Parser {
                    context: context.clone(),
                    message: error.to_string(),
                })?;

            match event {
                FfmpegEvent::LogEOF => break,
                FfmpegEvent::Error(message) => {
                    return Err(FfmpegError::Event { context, message });
                }
                event => {
                    if let FfmpegEvent::Log(_, content) = &event {
                        logs.push(content.clone());
                    }
                    if let (Some(reporter), Some(message)) =
                        (reporter, self.progress.message_for(&event))
                    {
                        reporter.update(&message).await;
                    }
                }
            }
        }

        let status = child.wait().await.map_err(|source| FfmpegError::Wait {
            context: context.clone(),
            source,
        })?;
        if !status.success() {
            return Err(FfmpegError::NonZeroExit { context, status });
        }

        Ok(JobOutput {
            output: self.output.clone(),
            status,
            logs,
        })
    }
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
    #[error("{context}: ffmpeg 异常退出（{status}）")]
    NonZeroExit { context: String, status: ExitStatus },
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
    /// Log lines parsed from ffmpeg stderr.
    pub logs: Vec<String>,
}

/// A best-effort RAII guard for a temporary file produced by ffmpeg.
///
/// `Drop` cannot await, so cleanup intentionally uses the synchronous standard
/// library call. This is only for artifacts that are deleted as soon as the job
/// that produced them is done; anything the caller keeps must not be guarded.
#[derive(Debug)]
pub struct TempArtifact {
    path: PathBuf,
}

impl TempArtifact {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl Drop for TempArtifact {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
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
    use async_ffmpeg_sidecar::event::{FfmpegProgress, LogLevel};
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

    #[test]
    fn progress_modes_have_distinct_basic_behavior() {
        let progress = sample_progress_event();
        assert_eq!(ProgressMode::None.message_for(&progress), None);
        assert_eq!(
            ProgressMode::Time.message_for(&progress),
            Some("00:00:01.00".to_owned())
        );

        let out_time = FfmpegEvent::Log(LogLevel::Info, "out_time_ms=1000000".to_owned());
        assert_eq!(ProgressMode::None.message_for(&out_time), None);
        assert_eq!(
            ProgressMode::OutTimeMs.message_for(&out_time),
            Some("1000000".to_owned())
        );
        assert_eq!(
            ProgressMode::Time.message_for(&FfmpegEvent::Log(
                LogLevel::Info,
                "out_time=00:00:01.00".to_owned(),
            )),
            Some("00:00:01.00".to_owned())
        );
        assert_eq!(
            ProgressMode::prefixed("压制中：").message_for(&progress),
            Some("压制中：00:00:01.00".to_owned())
        );
        assert_eq!(
            ProgressMode::labeled("正在转换视频格式... ", " (无损转换)").message_for(&progress),
            Some("正在转换视频格式... 00:00:01.00 (无损转换)".to_owned())
        );
    }

    #[tokio::test]
    async fn nonzero_exit_is_reported_as_structured_error() {
        let job = FfmpegJob::new().context("non-zero test");
        let result = job
            .run_with_command(test_command("exit 17"), None::<&RecordingReporter>)
            .await;

        match result {
            Err(FfmpegError::NonZeroExit { context, status }) => {
                assert_eq!(context, "non-zero test");
                assert!(!status.success());
            }
            other => panic!("expected non-zero exit error, got {other:?}"),
        }
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
