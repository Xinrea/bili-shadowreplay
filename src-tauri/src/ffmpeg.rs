
use ffmpeg_sidecar::{
  command::FfmpegCommand,
  event::{FfmpegEvent, LogLevel},
};

pub struct TranscodeConfig {
    pub input_path: String,
    pub input_format: String,
    pub output_path: String,
}


pub struct TranscodeResult {
    pub output_path: String,
}


pub fn transcode(work_dir: &str, config: TranscodeConfig) -> Result<TranscodeResult, String> {
    let input_path = config.input_path;
    let input_format = config.input_format;
    let output_path = config.output_path;

    println!("transcode task start: input_path: {}, output_path: {}", input_path, output_path);

    FfmpegCommand::new()
    .args([
      "-f", input_format.as_str(),
    ])
    .input(format!("{}/{}", work_dir, input_path))
    .args(["-c", "copy"])
    .args(["-y", format!("{}/{}", work_dir, output_path).as_str()])
    .spawn()
    .unwrap()
    .iter()
    .unwrap()
    .for_each(|e| match e {
      FfmpegEvent::Log(LogLevel::Error, e) => println!("Error: {}", e),
      FfmpegEvent::Progress(p) => println!("Progress: {}", p.time),
      _ => {}
    });

    println!("transcode task end: output_path: {}", output_path);

    Ok(TranscodeResult {
        output_path: format!("{}/{}", work_dir, output_path),
    })
}