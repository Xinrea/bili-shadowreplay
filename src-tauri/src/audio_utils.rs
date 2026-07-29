use std::path::Path;

use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};

/// Audio utility functions for Silero VAD and Cut & Merge segmentation.
///
/// A speech segment with start and end times in seconds.
#[derive(Debug, Clone)]
pub struct SpeechSegment {
    pub start: f64,
    pub end: f64,
}

pub fn silero_vad(
    samples: &[f32],
    sample_rate: u32,
    model_path: &Path,
) -> Result<Vec<SpeechSegment>, String> {
    if sample_rate != 16_000 {
        return Err(format!(
            "Silero VAD expects 16000 Hz audio, got {sample_rate} Hz"
        ));
    }

    let config = VadModelConfig {
        silero_vad: SileroVadModelConfig {
            model: Some(model_path.to_string_lossy().into_owned()),
            threshold: 0.5,
            min_silence_duration: 0.25,
            min_speech_duration: 0.25,
            window_size: 512,
            max_speech_duration: 28.0,
        },
        sample_rate: sample_rate as i32,
        num_threads: 1,
        provider: Some("cpu".to_string()),
        debug: false,
        ..Default::default()
    };
    let vad = VoiceActivityDetector::create(&config, 30.0)
        .ok_or_else(|| "Failed to create sherpa-onnx Silero VAD".to_string())?;

    let mut segments = Vec::new();
    let mut drain_segments = || {
        while let Some(segment) = vad.front() {
            let start = segment.start().max(0) as f64 / f64::from(sample_rate);
            let end = start + segment.n().max(0) as f64 / f64::from(sample_rate);
            drop(segment);
            vad.pop();
            if end > start {
                segments.push(SpeechSegment { start, end });
            }
        }
    };

    for chunk in samples.chunks(512) {
        vad.accept_waveform(chunk);
        drain_segments();
    }
    vad.flush();
    drain_segments();

    Ok(segments)
}

pub fn rms_energies(samples: &[f32], sample_rate: u32) -> (Vec<f64>, f64) {
    if samples.is_empty() || sample_rate == 0 {
        return (vec![], 0.01);
    }

    let frame_len = ((sample_rate as f64 * 0.025) as usize).max(1);
    let frame_step = ((sample_rate as f64 * 0.010) as usize).max(1);
    let mut energies = Vec::new();
    let mut pos = 0;
    while pos + frame_len <= samples.len() {
        let sum_sq: f64 = samples[pos..pos + frame_len]
            .iter()
            .map(|&sample| f64::from(sample) * f64::from(sample))
            .sum();
        energies.push((sum_sq / frame_len as f64).sqrt());
        pos += frame_step;
    }

    (energies, frame_step as f64 / sample_rate as f64)
}

/// Apply WhisperX Cut & Merge to speech segments.
///
/// - **Cut**: segments longer than `cut_max` seconds are split at the
///   point of minimum energy within the window `[cut_max/2, cut_max]`.
/// - **Merge**: adjacent segments whose combined duration ≤ `merge_max`
///   are merged together.
///
/// `energies` is the per-frame RMS energy returned by [`rms_energies`].
/// `frame_sec` is the duration of one frame in seconds.
pub fn cut_and_merge(
    segments: &[SpeechSegment],
    energies: &[f64],
    frame_sec: f64,
    cut_max: f64,
    merge_max: f64,
) -> Vec<SpeechSegment> {
    // Step 1: Cut long segments
    let mut cut: Vec<SpeechSegment> = Vec::new();
    for seg in segments {
        let duration = seg.end - seg.start;
        if duration <= cut_max {
            cut.push(seg.clone());
        } else {
            // Find the minimum-energy point in [cut_max/2, cut_max] window
            let mut pos = seg.start;
            while pos + cut_max < seg.end {
                let window_start = pos + cut_max / 2.0;
                let window_end = (pos + cut_max).min(seg.end);

                let frame_start = (window_start / frame_sec) as usize;
                let frame_end = ((window_end / frame_sec) as usize).min(energies.len());

                let mut min_idx = frame_start;
                let mut min_energy = f64::MAX;
                for fi in frame_start..frame_end {
                    if fi < energies.len() && energies[fi] < min_energy {
                        min_energy = energies[fi];
                        min_idx = fi;
                    }
                }

                let cut_point = min_idx as f64 * frame_sec;
                if cut_point <= pos {
                    // Degenerate case: force cut at the midpoint
                    let mid = pos + cut_max / 2.0;
                    cut.push(SpeechSegment {
                        start: pos,
                        end: mid.min(seg.end),
                    });
                    pos = mid.min(seg.end);
                } else {
                    cut.push(SpeechSegment {
                        start: pos,
                        end: cut_point,
                    });
                    pos = cut_point;
                }
            }
            // Remaining tail (≤ cut_max)
            if pos < seg.end {
                cut.push(SpeechSegment {
                    start: pos,
                    end: seg.end,
                });
            }
        }
    }

    // Step 2: Merge adjacent short segments
    let mut merged: Vec<SpeechSegment> = Vec::new();
    for seg in cut {
        if let Some(last) = merged.last_mut() {
            let combined = seg.end - last.start;
            if combined <= merge_max {
                last.end = seg.end;
                continue;
            }
        }
        merged.push(seg);
    }

    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rms_energies_silence() {
        let samples = vec![0.0f32; 16000]; // 1 second of silence
        let (energies, frame_sec) = rms_energies(&samples, 16000);
        assert!(!energies.is_empty());
        assert!(energies.iter().all(|energy| *energy == 0.0));
        assert!((frame_sec - 0.01).abs() < f64::EPSILON);
    }

    #[test]
    fn test_rms_energies_speech() {
        // 1 second of loud sine wave
        let samples: Vec<f32> = (0..16000)
            .map(|i| (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / 16000.0).sin() * 0.8)
            .collect();
        let (energies, _) = rms_energies(&samples, 16000);
        assert!(energies.iter().all(|energy| *energy > 0.1));
    }

    #[test]
    #[ignore = "requires SILERO_VAD_TEST_MODEL and SILERO_VAD_TEST_AUDIO"]
    fn test_silero_vad_with_external_audio() {
        let model = std::env::var("SILERO_VAD_TEST_MODEL").unwrap();
        let audio = std::env::var("SILERO_VAD_TEST_AUDIO").unwrap();
        let wave = sherpa_onnx::Wave::read(&audio).unwrap();
        let segments =
            silero_vad(wave.samples(), wave.sample_rate() as u32, Path::new(&model)).unwrap();
        let speech_duration: f64 = segments
            .iter()
            .map(|segment| segment.end - segment.start)
            .sum();
        let (energies, frame_sec) = rms_energies(wave.samples(), wave.sample_rate() as u32);
        let chunks = cut_and_merge(&segments, &energies, frame_sec, 30.0, 10.0);

        println!("Detected {} speech segments", segments.len());
        println!("Total speech duration: {speech_duration:.2}s");
        println!("Cut & Merge produced {} chunks", chunks.len());

        assert!(segments.len() > 1);
        assert!(chunks.len() > 1);
        assert!(speech_duration > 10.0);
        assert!(segments
            .iter()
            .all(|segment| segment.end - segment.start <= 28.1));
    }

    #[test]
    fn test_cut_and_merge_noop() {
        let segments = vec![SpeechSegment {
            start: 0.0,
            end: 10.0,
        }];
        let energies = vec![0.1; 1000];
        let result = cut_and_merge(&segments, &energies, 0.01, 30.0, 10.0);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 0.0);
        assert_eq!(result[0].end, 10.0);
    }

    #[test]
    fn test_cut_and_merge_split_long() {
        let segments = vec![SpeechSegment {
            start: 0.0,
            end: 90.0,
        }];
        // Uniform energy — cuts at midpoint of [15,30] = 22.5, then again
        let energies = vec![0.1; 10000];
        let result = cut_and_merge(&segments, &energies, 0.01, 30.0, 10.0);
        // Should split into ~3 segments of ~30s each
        assert!(result.len() >= 3);
        for seg in &result {
            assert!(seg.end - seg.start <= 30.0 + 0.01);
        }
    }

    #[test]
    fn test_cut_and_merge_adjacent_no_merge() {
        let segments = vec![
            SpeechSegment {
                start: 0.0,
                end: 10.0,
            },
            SpeechSegment {
                start: 11.0, // 1s gap
                end: 18.0,
            },
        ];
        // Merging disabled — segments should remain separate
        let energies = vec![0.1; 2000];
        let result = cut_and_merge(&segments, &energies, 0.01, 30.0, 10.0);
        assert_eq!(result.len(), 2);
    }
}
