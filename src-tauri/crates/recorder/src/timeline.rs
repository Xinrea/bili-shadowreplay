use std::collections::HashMap;
use std::io;
use std::path::Path;

use recording_storage::RecordingStorage;
use serde::{Deserialize, Serialize};

use crate::danmu::DanmuEntry;

pub const SKIPPED_AD_RANGES_FILE_NAME: &str = "skipped_ad_ranges.jsonl";

/// A wall-clock interval removed from a recorded HLS playlist.
///
/// `id` is Twitch's `EXT-X-DATERANGE` ID. It lets a later playlist reload
/// update a provisional `PLANNED-DURATION` with the final `END-DATE`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct AdTimeRange {
    pub id: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

impl AdTimeRange {
    pub fn is_valid(&self) -> bool {
        self.end_ms > self.start_ms
    }
}

/// Append a skipped ad interval to the archive sidecar.
///
/// The file is JSON Lines so a process interruption cannot discard earlier
/// ranges. Readers keep the latest record for each DATERANGE ID and start time.
pub async fn append_skipped_ad_range(work_dir: &Path, range: &AdTimeRange) -> io::Result<()> {
    append_skipped_ad_range_to_storage(&RecordingStorage::local(work_dir), range).await
}

pub async fn append_skipped_ad_range_to_storage(
    storage: &RecordingStorage,
    range: &AdTimeRange,
) -> io::Result<()> {
    if !range.is_valid() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ad time range must have a positive duration",
        ));
    }
    let mut json = serde_json::to_vec(range)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    json.push(b'\n');
    let log = storage.open_log(SKIPPED_AD_RANGES_FILE_NAME).await?;
    log.append(&json).await?;
    log.flush().await
}

/// Load the latest persisted interval for each ad DATERANGE ID and start time.
pub async fn load_skipped_ad_ranges(work_dir: &Path) -> io::Result<Vec<AdTimeRange>> {
    load_skipped_ad_ranges_from_storage(&RecordingStorage::local(work_dir)).await
}

pub async fn load_skipped_ad_ranges_from_storage(
    storage: &RecordingStorage,
) -> io::Result<Vec<AdTimeRange>> {
    let contents = match storage.read_log(SKIPPED_AD_RANGES_FILE_NAME).await {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut latest_by_id_and_start = HashMap::new();
    for line in contents.split(|byte| *byte == b'\n') {
        if let Ok(range) = serde_json::from_slice::<AdTimeRange>(line) {
            if range.is_valid() {
                latest_by_id_and_start.insert((range.id.clone(), range.start_ms), range);
            }
        }
    }
    let mut ranges: Vec<_> = latest_by_id_and_start.into_values().collect();
    ranges.sort_by(|left, right| {
        (left.start_ms, left.end_ms, &left.id).cmp(&(right.start_ms, right.end_ms, &right.id))
    });
    Ok(ranges)
}

/// Remove danmaku that fell inside removed ad intervals and compress later
/// absolute timestamps onto the recorded video's continuous timeline.
///
/// `timeline_start_ms` is the first retained segment's PROGRAM-DATE-TIME. Ad
/// intervals before that point are still used to filter their own messages, but
/// do not shift the archive because they precede its timeline origin.
pub fn align_danmus_to_recording(
    danmus: Vec<DanmuEntry>,
    timeline_start_ms: i64,
    ranges: &[AdTimeRange],
) -> Vec<DanmuEntry> {
    let merged_ranges = merge_intervals(ranges);
    if merged_ranges.is_empty() {
        return danmus;
    }
    let intervals_to_shift: Vec<_> = merged_ranges
        .iter()
        .filter(|interval| interval.1 > timeline_start_ms)
        .map(|&(start_ms, end_ms)| (start_ms.max(timeline_start_ms), end_ms))
        .collect();

    danmus
        .into_iter()
        .filter_map(|mut danmu| {
            if merged_ranges
                .iter()
                .any(|(start_ms, end_ms)| danmu.ts >= *start_ms && danmu.ts < *end_ms)
            {
                return None;
            }

            if danmu.ts >= timeline_start_ms {
                let removed_duration = intervals_to_shift
                    .iter()
                    .take_while(|interval| interval.1 <= danmu.ts)
                    .fold(0_i64, |total, &(start_ms, end_ms)| {
                        total.saturating_add(end_ms.saturating_sub(start_ms))
                    });
                danmu.ts = danmu.ts.saturating_sub(removed_duration);
            }
            Some(danmu)
        })
        .collect()
}

fn merge_intervals(ranges: &[AdTimeRange]) -> Vec<(i64, i64)> {
    let mut intervals: Vec<_> = ranges
        .iter()
        .filter(|range| range.is_valid())
        .map(|range| (range.start_ms, range.end_ms))
        .collect();
    intervals.sort_unstable();

    let mut merged: Vec<(i64, i64)> = Vec::with_capacity(intervals.len());
    for (start_ms, end_ms) in intervals {
        if let Some((_, last_end_ms)) = merged.last_mut() {
            if start_ms <= *last_end_ms {
                *last_end_ms = (*last_end_ms).max(end_ms);
                continue;
            }
        }
        merged.push((start_ms, end_ms));
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMELINE_START_MS: i64 = 1_700_000_000_000;

    #[tokio::test]
    async fn storage_resumes_ranges_and_ignores_invalid_rows() {
        let dir = tempfile::tempdir().unwrap();
        let storage = RecordingStorage::local(dir.path());
        assert!(load_skipped_ad_ranges_from_storage(&storage)
            .await
            .unwrap()
            .is_empty());
        storage
            .write(
                SKIPPED_AD_RANGES_FILE_NAME,
                b"invalid\n{\"id\":\"invalid\",\"start_ms\":10,\"end_ms\":10}\n",
            )
            .await
            .unwrap();
        let first = range(100, 200);
        append_skipped_ad_range_to_storage(&storage, &first)
            .await
            .unwrap();
        let updated = AdTimeRange {
            end_ms: 300,
            ..first
        };
        // The path wrapper and storage API share the same sidecar.
        append_skipped_ad_range(dir.path(), &updated).await.unwrap();
        let earlier = range(0, 10);
        append_skipped_ad_range_to_storage(&storage, &earlier)
            .await
            .unwrap();
        assert_eq!(
            load_skipped_ad_ranges_from_storage(&storage).await.unwrap(),
            vec![earlier, updated]
        );
        let bytes = storage.read(SKIPPED_AD_RANGES_FILE_NAME).await.unwrap();
        assert_eq!(bytes.iter().filter(|&&byte| byte == b'\n').count(), 5);
        assert_eq!(bytes.last(), Some(&b'\n'));
    }

    #[tokio::test]
    async fn storage_reports_invalid_ranges_and_io_failures() {
        let dir = tempfile::tempdir().unwrap();
        let storage = RecordingStorage::local(dir.path());
        let error = append_skipped_ad_range_to_storage(&storage, &range(10, 10))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            storage
                .read(SKIPPED_AD_RANGES_FILE_NAME)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        std::fs::create_dir(dir.path().join(SKIPPED_AD_RANGES_FILE_NAME)).unwrap();
        assert!(load_skipped_ad_ranges_from_storage(&storage).await.is_err());
        assert!(append_skipped_ad_range_to_storage(&storage, &range(10, 20))
            .await
            .is_err());
        assert!(load_skipped_ad_ranges(dir.path()).await.is_err());
        assert!(append_skipped_ad_range(dir.path(), &range(10, 20))
            .await
            .is_err());
    }

    fn danmu(ts: i64, content: &str) -> DanmuEntry {
        DanmuEntry {
            ts,
            event_type: "danmu".to_string(),
            content: content.to_string(),
            emoji_content: None,
            user_name: None,
            price: None,
            sc_duration: None,
        }
    }

    fn range(start_ms: i64, end_ms: i64) -> AdTimeRange {
        AdTimeRange {
            id: format!("stitched-ad-{start_ms}"),
            start_ms,
            end_ms,
        }
    }

    #[test]
    fn drops_ad_danmu_and_compacts_later_timestamps() {
        let aligned = align_danmus_to_recording(
            vec![
                danmu(TIMELINE_START_MS + 2_000, "before"),
                danmu(TIMELINE_START_MS + 6_000, "during ad"),
                danmu(TIMELINE_START_MS + 8_000, "after"),
            ],
            TIMELINE_START_MS,
            &[range(TIMELINE_START_MS + 4_000, TIMELINE_START_MS + 8_000)],
        );

        assert_eq!(
            aligned
                .iter()
                .map(|entry| (entry.ts - TIMELINE_START_MS, entry.content.as_str()))
                .collect::<Vec<_>>(),
            vec![(2_000, "before"), (4_000, "after")]
        );
    }

    #[test]
    fn initial_ad_is_filtered_without_shifting_recording_start() {
        let aligned = align_danmus_to_recording(
            vec![
                danmu(TIMELINE_START_MS - 500, "initial ad"),
                danmu(TIMELINE_START_MS + 1_000, "first program segment"),
            ],
            TIMELINE_START_MS,
            &[range(TIMELINE_START_MS - 4_000, TIMELINE_START_MS)],
        );

        assert_eq!(aligned.len(), 1);
        assert_eq!(aligned[0].ts, TIMELINE_START_MS + 1_000);
    }

    #[test]
    fn multiple_ad_breaks_accumulate_removed_duration() {
        let aligned = align_danmus_to_recording(
            vec![danmu(TIMELINE_START_MS + 20_000, "after both ads")],
            TIMELINE_START_MS,
            &[
                range(TIMELINE_START_MS + 4_000, TIMELINE_START_MS + 8_000),
                range(TIMELINE_START_MS + 12_000, TIMELINE_START_MS + 15_000),
            ],
        );

        assert_eq!(aligned[0].ts, TIMELINE_START_MS + 13_000);
    }

    #[test]
    fn overlapping_ranges_do_not_double_compress_timestamps() {
        let aligned = align_danmus_to_recording(
            vec![danmu(TIMELINE_START_MS + 12_000, "after")],
            TIMELINE_START_MS,
            &[
                range(TIMELINE_START_MS + 4_000, TIMELINE_START_MS + 8_000),
                range(TIMELINE_START_MS + 6_000, TIMELINE_START_MS + 10_000),
            ],
        );

        assert_eq!(aligned[0].ts, TIMELINE_START_MS + 6_000);
    }

    #[tokio::test]
    async fn skipped_ad_ranges_round_trip_and_latest_id_wins() {
        let work_dir = std::env::temp_dir().join(format!(
            "bili-shadowreplay-skipped-ad-ranges-{}",
            uuid::Uuid::new_v4()
        ));
        tokio::fs::create_dir_all(&work_dir).await.unwrap();
        let first = AdTimeRange {
            id: "stitched-ad-1".to_string(),
            start_ms: 1_000,
            end_ms: 5_000,
        };
        append_skipped_ad_range(&work_dir, &first).await.unwrap();
        let updated = AdTimeRange {
            end_ms: 4_000,
            ..first.clone()
        };
        append_skipped_ad_range(&work_dir, &updated).await.unwrap();

        assert_eq!(
            load_skipped_ad_ranges(&work_dir).await.unwrap(),
            vec![updated]
        );
        tokio::fs::remove_dir_all(work_dir).await.unwrap();
    }
}
