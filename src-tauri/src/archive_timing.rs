use chrono::DateTime;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::SystemTime,
};
use tokio::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveTiming {
    /// Actual wall-clock coverage, in Unix milliseconds, independent of media duration.
    pub started_at: i64,
    pub ended_at: i64,
}

fn duration_ms(seconds: f64) -> i64 {
    if seconds.is_finite() && seconds > 0.0 {
        (seconds * 1000.0).round() as i64
    } else {
        0
    }
}

fn playlist_timing(bytes: &[u8]) -> Option<ArchiveTiming> {
    let (_, playlist) = m3u8_rs::parse_media_playlist(bytes).ok()?;
    let mut started_at = None;
    let mut ended_at: Option<i64> = None;
    let mut cursor = None;
    let mut preceding_duration = 0_i64;
    for segment in playlist.segments {
        let explicit_time = segment
            .program_date_time
            .map(|time| time.timestamp_millis())
            .or_else(|| {
                // Older manifests used the nonstandard X-PROGRAM-DATE-TIME tag.
                segment
                    .unknown_tags
                    .iter()
                    .find(|tag| tag.tag == "X-PROGRAM-DATE-TIME")
                    .and_then(|tag| tag.rest.as_ref())
                    .and_then(|value| {
                        DateTime::parse_from_rfc3339(value)
                            .or_else(|_| DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f%z"))
                            .ok()
                    })
                    .map(|time| time.timestamp_millis())
            });
        // A discontinuity without a new clock anchor cannot establish actual coverage.
        if segment.discontinuity && explicit_time.is_none() {
            return None;
        }
        let duration = duration_ms(segment.duration as f64);
        if let Some(time) = explicit_time.or(cursor) {
            started_at.get_or_insert_with(|| time.saturating_sub(preceding_duration));
            let end = time.saturating_add(duration);
            ended_at = Some(ended_at.map_or(end, |previous| previous.max(end)));
            cursor = Some(end);
        } else {
            preceding_duration = preceding_duration.saturating_add(duration);
        }
    }
    Some(ArchiveTiming {
        started_at: started_at?,
        ended_at: ended_at?,
    })
}

fn entries_timing(bytes: &[u8]) -> Option<ArchiveTiming> {
    let mut timing: Option<ArchiveTiming> = None;
    for line in std::str::from_utf8(bytes).ok()?.lines() {
        let Ok(entry) = recorder::entry::TsEntry::from(line) else {
            continue;
        };
        if entry.is_header {
            continue;
        }
        // Legacy Douyin entries use seconds; other entries use milliseconds.
        let start = if entry.ts > 10_000_000_000 {
            entry.ts
        } else {
            entry.ts.saturating_mul(1000)
        };
        let end = start.saturating_add(duration_ms(entry.length));
        timing = Some(match timing {
            Some(previous) => ArchiveTiming {
                started_at: previous.started_at.min(start),
                ended_at: previous.ended_at.max(end),
            },
            None => ArchiveTiming {
                started_at: start,
                ended_at: end,
            },
        });
    }
    timing
}

struct CachedTiming {
    size: u64,
    modified: Option<SystemTime>,
    timing: Option<ArchiveTiming>,
}

/// Historical playlists are parsed once; an active recording invalidates by file metadata.
#[derive(Default)]
pub struct ArchiveTimingCache(Mutex<HashMap<PathBuf, CachedTiming>>);

impl ArchiveTimingCache {
    pub async fn get(&self, directory: &Path) -> Option<ArchiveTiming> {
        for (name, parse) in [
            (
                "playlist.m3u8",
                playlist_timing as fn(&[u8]) -> Option<ArchiveTiming>,
            ),
            ("entries.log", entries_timing),
        ] {
            let path = directory.join(name);
            let Ok(metadata) = tokio::fs::metadata(&path).await else {
                self.0.lock().await.remove(&path);
                continue;
            };
            let size = metadata.len();
            let modified = metadata.modified().ok();
            let cached = self
                .0
                .lock()
                .await
                .get(&path)
                .filter(|cached| cached.size == size && cached.modified == modified)
                .map(|cached| cached.timing);
            let timing = match cached {
                Some(timing) => timing,
                None => {
                    let Ok(bytes) = tokio::fs::read(&path).await else {
                        continue;
                    };
                    let timing = parse(&bytes);
                    self.0.lock().await.insert(
                        path,
                        CachedTiming {
                            size,
                            modified,
                            timing,
                        },
                    );
                    timing
                }
            };
            if timing.is_some() {
                return timing;
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: i64 = 1_793_527_200_000; // 2026-11-01T10:00:00Z
    const GAP_PLAYLIST: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:1800\n#EXT-X-PROGRAM-DATE-TIME:2026-11-01T10:00:00Z\n#EXTINF:1800,\n0.ts\n#EXT-X-DISCONTINUITY\n#EXT-X-PROGRAM-DATE-TIME:2026-11-01T10:40:00Z\n#EXTINF:1200,\n1.ts\n";

    #[test]
    fn skipped_ads_preserve_wall_clock_coverage_instead_of_media_duration() {
        let timing = playlist_timing(GAP_PLAYLIST.as_bytes()).unwrap();
        assert_eq!(
            timing,
            ArchiveTiming {
                started_at: START,
                ended_at: START + 3_600_000
            }
        );
    }

    #[test]
    fn sparse_clock_anchors_cover_segments_before_and_after_the_anchor() {
        let bytes = b"#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXTINF:10,\n0.ts\n#EXT-X-PROGRAM-DATE-TIME:2026-11-01T10:00:00Z\n#EXTINF:2.5,\n1.ts\n#EXTINF:7.5,\n2.ts\n";
        assert_eq!(
            playlist_timing(bytes),
            Some(ArchiveTiming {
                started_at: START - 10_000,
                ended_at: START + 10_000
            })
        );
        assert_eq!(playlist_timing(b"#EXTM3U\n#EXTINF:10,\n0.ts\n"), None);
    }

    #[test]
    fn legacy_entries_keep_gaps_and_ignore_headers() {
        let bytes = format!(
            "init|0|0|100|1|true\n0.ts|1|10|100|{}|false\nbad line\n1.ts|2|5|100|{}|false\n",
            START / 1000,
            START + 60_000
        );
        assert_eq!(
            entries_timing(bytes.as_bytes()),
            Some(ArchiveTiming {
                started_at: START,
                ended_at: START + 65_000
            })
        );
    }

    #[tokio::test]
    async fn cached_timing_refreshes_as_recording_grows_and_handles_missing_archives() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ArchiveTimingCache::default();
        assert_eq!(cache.get(dir.path()).await, None);
        let path = dir.path().join("playlist.m3u8");
        tokio::fs::write(&path, GAP_PLAYLIST).await.unwrap();
        assert_eq!(
            cache.get(dir.path()).await.unwrap().ended_at,
            START + 3_600_000
        );
        assert_eq!(
            cache.get(dir.path()).await.unwrap().ended_at,
            START + 3_600_000
        );
        tokio::fs::write(&path, format!("{GAP_PLAYLIST}#EXTINF:10,\n2.ts\n"))
            .await
            .unwrap();
        assert_eq!(
            cache.get(dir.path()).await.unwrap().ended_at,
            START + 3_610_000
        );
        tokio::fs::remove_file(path).await.unwrap();
        assert_eq!(cache.get(dir.path()).await, None);
    }
}
