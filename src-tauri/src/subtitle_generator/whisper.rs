use async_trait::async_trait;

use async_std::sync::{Arc, RwLock};
use std::path::Path;
use tokio::io::AsyncWriteExt;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::SubtitleGenerator;

#[derive(Clone)]
pub struct WhisperCPP {
    ctx: Arc<RwLock<WhisperContext>>,
    prompt: String,
}

pub async fn new(model: &Path, prompt: &str) -> Result<WhisperCPP, String> {
    let ctx = WhisperContext::new_with_params(
        model.to_str().unwrap(),
        WhisperContextParameters::default(),
    )
    .expect("failed to load model");

    Ok(WhisperCPP {
        ctx: Arc::new(RwLock::new(ctx)),
        prompt: prompt.to_string(),
    })
}

#[async_trait]
impl SubtitleGenerator for WhisperCPP {
    async fn generate_subtitle(
        &self,
        audio_path: &Path,
        output_path: &Path,
    ) -> Result<String, String> {
        log::info!("Generating subtitle for {:?}", audio_path);
        let start_time = std::time::Instant::now();
        let samples: Vec<i16> = hound::WavReader::open(audio_path)
            .unwrap()
            .into_samples::<i16>()
            .map(|x| x.unwrap())
            .collect();

        let mut state = self
            .ctx
            .read()
            .await
            .create_state()
            .expect("failed to create state");

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        // and set the language to translate to to auto
        params.set_language(None);
        params.set_initial_prompt(self.prompt.as_str());

        // we also explicitly disable anything that prints to stdout
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        let mut inter_samples = vec![Default::default(); samples.len()];

        whisper_rs::convert_integer_to_float_audio(&samples, &mut inter_samples)
            .expect("failed to convert audio data");
        let samples = whisper_rs::convert_stereo_to_mono_audio(&inter_samples)
            .expect("failed to convert audio data");

        state
            .full(params, &samples[..])
            .expect("failed to run model");

        // open the output file
        let mut output_file = tokio::fs::File::create(output_path)
            .await
            .expect("failed to create output file");
        // fetch the results
        let num_segments = state
            .full_n_segments()
            .expect("failed to get number of segments");
        let mut subtitle = String::new();
        for i in 0..num_segments {
            let segment = state
                .full_get_segment_text(i)
                .expect("failed to get segment");
            let start_timestamp = state
                .full_get_segment_t0(i)
                .expect("failed to get segment start timestamp");
            let end_timestamp = state
                .full_get_segment_t1(i)
                .expect("failed to get segment end timestamp");

            let format_time = |timestamp: f64| {
                let hours = (timestamp / 3600.0).floor();
                let minutes = ((timestamp - hours * 3600.0) / 60.0).floor();
                let seconds = timestamp - hours * 3600.0 - minutes * 60.0;
                format!("{:02}:{:02}:{:06.3}", hours, minutes, seconds).replace(".", ",")
            };

            let line = format!(
                "{}\n{} --> {}\n{}\n\n",
                i + 1,
                format_time(start_timestamp as f64 / 100.0),
                format_time(end_timestamp as f64 / 100.0),
                segment,
            );

            subtitle.push_str(&line);
        }

        output_file
            .write_all(subtitle.as_bytes())
            .await
            .expect("failed to write to output file");

        log::info!("Subtitle generated: {:?}", output_path);
        log::info!("Time taken: {} seconds", start_time.elapsed().as_secs_f64());

        Ok(subtitle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "need whisper-cli"]
    async fn create_whisper_cpp() {
        let result = new(Path::new("tests/model/ggml-model-whisper-tiny.bin"), "").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[ignore = "need large model"]
    async fn process_by_whisper_cpp() {
        let whisper = new(
            Path::new("tests/model/ggml-model-whisper-large-q5_0.bin"),
            "",
        )
        .await
        .unwrap();
        let audio_path = Path::new("tests/audio/test.wav");
        let output_path = Path::new("tests/audio/test.srt");
        let result = whisper.generate_subtitle(audio_path, output_path).await;
        assert!(result.is_ok());
    }
}
