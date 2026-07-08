/// Audio utility functions: VAD (Voice Activity Detection) and
/// Cut & Merge segmentation for long-form audio preprocessing.
///
/// Based on the WhisperX paper (Bain et al., 2023):
///   - VAD to find speech boundaries at natural silence points
///   - Cut segments > 30s at minimum-energy points
///   - Merge adjacent short segments ≤ 30s
///
/// A speech segment with start and end times in seconds.
#[derive(Debug, Clone)]
pub struct SpeechSegment {
    pub start: f64,
    pub end: f64,
}

/// Energy-based VAD on 16kHz mono f32 PCM samples.
///
/// Returns speech segments sorted by start time.
#[allow(dead_code)]
pub fn energy_vad(samples: &[f32], sample_rate: u32) -> Vec<SpeechSegment> {
    energy_vad_with_energies(samples, sample_rate).0
}

/// Energy-based VAD on 16kHz mono f32 PCM samples.
///
/// Returns speech segments, per-frame RMS energies and the frame duration in seconds.
pub fn energy_vad_with_energies(
    samples: &[f32],
    sample_rate: u32,
) -> (Vec<SpeechSegment>, Vec<f64>, f64) {
    if samples.is_empty() {
        return (vec![], vec![], 0.01);
    }

    let frame_len = ((sample_rate as f64 * 0.025) as usize).max(1); // 25ms frames
    let frame_step = ((sample_rate as f64 * 0.010) as usize).max(1); // 10ms hop

    // Compute RMS energy per frame
    let mut energies: Vec<f64> = Vec::new();
    let mut pos = 0;
    while pos + frame_len <= samples.len() {
        let sum_sq: f64 = samples[pos..pos + frame_len]
            .iter()
            .map(|&s| (s as f64) * (s as f64))
            .sum();
        let rms = (sum_sq / frame_len as f64).sqrt();
        energies.push(rms);
        pos += frame_step;
    }

    if energies.is_empty() {
        return (vec![], energies, frame_step as f64 / sample_rate as f64);
    }

    // Dynamic threshold: use a fraction of the median energy of
    // frames that are above the absolute silence floor.
    let silence_floor = 1e-4;
    let active: Vec<f64> = energies
        .iter()
        .copied()
        .filter(|&e| e > silence_floor)
        .collect();

    if active.is_empty() {
        return (vec![], energies, frame_step as f64 / sample_rate as f64);
    }

    let mut sorted = active.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median_energy = sorted[sorted.len() / 2];

    // Use a stricter onset threshold to avoid treating background
    // noise/music as speech.  silence_floor * 20 is a hard floor.
    let onset_threshold = (median_energy * 0.5).max(silence_floor * 20.0);

    // Classify each frame as speech (true) or silence (false)
    let frame_sec = frame_step as f64 / sample_rate as f64;
    let mut is_speech: Vec<bool> = energies.iter().map(|&e| e > onset_threshold).collect();

    // Temporal smoothing: fill short gaps inside phrases and remove only
    // extremely short speech bursts. The previous thresholds were too
    // aggressive for conversational clips and dropped many real utterances.
    let min_speech_frames = ((0.15 / frame_sec) as usize).max(1);
    let max_gap_frames = ((0.20 / frame_sec) as usize).max(1);

    // Remove short speech bursts
    let mut i = 0;
    while i < is_speech.len() {
        if is_speech[i] {
            let mut j = i;
            while j < is_speech.len() && is_speech[j] {
                j += 1;
            }
            if j - i < min_speech_frames {
                for item in is_speech.iter_mut().take(j).skip(i) {
                    *item = false;
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }

    // Fill short gaps
    i = 0;
    while i < is_speech.len() {
        if !is_speech[i] {
            let mut j = i;
            while j < is_speech.len() && !is_speech[j] {
                j += 1;
            }
            if j - i < max_gap_frames && i > 0 && j < is_speech.len() {
                for item in is_speech.iter_mut().take(j).skip(i) {
                    *item = true;
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }

    // Extract contiguous speech segments
    let mut segments = Vec::new();
    i = 0;
    while i < is_speech.len() {
        if is_speech[i] {
            let start = i as f64 * frame_sec;
            let mut j = i;
            while j < is_speech.len() && is_speech[j] {
                j += 1;
            }
            let end = j as f64 * frame_sec;
            // Drop only clearly spurious segments. Conversational particles
            // and clipped short replies are often well below 500ms.
            if end - start >= 0.25 {
                segments.push(SpeechSegment { start, end });
            }
            i = j;
        } else {
            i += 1;
        }
    }

    // Merge adjacent segments separated by < 0.5s of silence
    let mut merged: Vec<SpeechSegment> = Vec::new();
    for seg in segments {
        if let Some(last) = merged.last_mut() {
            if seg.start - last.end < 0.5 {
                last.end = seg.end;
                continue;
            }
        }
        merged.push(seg);
    }

    // Trim leading/trailing silence from each segment: re-check the
    // first and last 500ms and shrink the boundary if energy is below
    // half the onset threshold.
    let trim_secs = 0.5;
    let trim_threshold = onset_threshold * 0.5;
    for seg in &mut merged {
        // Trim leading silence
        let frame_from = (seg.start / frame_sec) as usize;
        let frame_trim_end = ((seg.start + trim_secs).min(seg.end) / frame_sec) as usize;
        let mut new_start = seg.start;
        for (fi, &energy) in energies
            .iter()
            .enumerate()
            .skip(frame_from)
            .take(frame_trim_end.saturating_sub(frame_from))
        {
            if energy > trim_threshold {
                new_start = fi as f64 * frame_sec;
                break;
            }
        }
        // Trim trailing silence
        let frame_to = (seg.end / frame_sec) as usize;
        let frame_trim_start = ((seg.end - trim_secs).max(seg.start) / frame_sec) as usize;
        let mut new_end = seg.end;
        for (fi, &energy) in energies
            .iter()
            .enumerate()
            .skip(frame_trim_start)
            .take(frame_to.saturating_sub(frame_trim_start))
        {
            if energy > trim_threshold {
                new_end = (fi + 1) as f64 * frame_sec;
            }
            // Continue to the end — we want the LAST frame above threshold
        }
        // Only apply if the trim doesn't collapse the segment entirely
        if new_end - new_start >= 0.3 {
            seg.start = new_start;
            seg.end = new_end;
        }
    }

    // Re-filter: drop any segments that became too short after trimming.
    merged.retain(|s| s.end - s.start >= 0.25);

    (merged, energies, frame_sec)
}

/// Apply WhisperX Cut & Merge to speech segments.
///
/// - **Cut**: segments longer than `cut_max` seconds are split at the
///   point of minimum energy within the window `[cut_max/2, cut_max]`.
/// - **Merge**: adjacent segments whose combined duration ≤ `merge_max`
///   are merged together.
///
/// `energies` is the per-frame RMS energy (same as from `energy_vad`).
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
    fn test_energy_vad_silence() {
        let samples = vec![0.0f32; 16000]; // 1 second of silence
        let segments = energy_vad(&samples, 16000);
        assert!(segments.is_empty());
    }

    #[test]
    fn test_energy_vad_speech() {
        // 1 second of loud sine wave
        let samples: Vec<f32> = (0..16000)
            .map(|i| (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / 16000.0).sin() * 0.8)
            .collect();
        let segments = energy_vad(&samples, 16000);
        // Should detect one continuous speech segment
        assert_eq!(segments.len(), 1);
        assert!(segments[0].start < 0.1);
        assert!(segments[0].end > 0.9);
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
