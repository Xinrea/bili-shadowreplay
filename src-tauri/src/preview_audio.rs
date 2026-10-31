//! Sound for the project editor's preview, cut into chunks.
//!
//! The preview plays every sound of a project through Web Audio, apart from
//! its videos, which only show pictures: it schedules these chunks back to
//! back on its own clock, so what is heard never depends on a video keeping in
//! step. A chunk is [`CHUNK_SECONDS`] of a file under the output directory, on
//! a fixed grid from its start, as 48 kHz stereo 16-bit FLAC, holding exactly
//! the samples a decode of the whole file has there, so chunks join sample for
//! sample. Each is cut once and kept in `<cache>/preview_audio/`, for a file
//! as long as it is unchanged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::ffmpeg::audio::{extract_preview_audio, probe_audio_timing, AudioTiming};
use crate::safe_path::join_relative;

pub const CHUNK_SECONDS: f64 = 10.0;
/// Changed when chunks are cut otherwise, so those cut before are not used.
/// 2: frames of a fixed size, which WebKit decodes, at the samples a decode of
/// the whole file has.
const CHUNK_FORMAT: u8 = 2;
/// Decoded before each chunk and dropped, so it starts as in the whole file.
const PRE_ROLL_SECONDS: f64 = 1.0;
/// Beyond about 11 days into a file.
const MAX_CHUNK: u32 = 100_000;
const CACHE_DIR: &str = "preview_audio";
/// Chunks of a file not cut again for this long are removed.
const KEEP_FOR: Duration = Duration::from_secs(3 * 24 * 3600);
/// How often the cache is looked through for those.
const PRUNE_EVERY_SECS: u64 = 3600;
static LAST_PRUNE: AtomicU64 = AtomicU64::new(0);
/// The timing of each file's sound, by the directory of its chunks.
static TIMINGS: LazyLock<Mutex<HashMap<PathBuf, Option<AudioTiming>>>> =
    LazyLock::new(Default::default);

#[derive(Debug, Deserialize)]
pub struct ChunkQuery {
    /// Relative to the output directory, `/`-separated.
    pub file: String,
    pub chunk: u32,
}

/// The directory of a file's chunks, named after it as it is now: a file
/// replaced under the same name gets new ones.
fn chunk_dir(cache: &Path, file: &str, metadata: &std::fs::Metadata) -> PathBuf {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |time| time.as_nanos());
    let mut hasher = Sha256::new();
    hasher.update([CHUNK_FORMAT]);
    hasher.update(file.as_bytes());
    hasher.update([0]);
    hasher.update(metadata.len().to_le_bytes());
    hasher.update(modified.to_le_bytes());
    let key: String = hasher
        .finalize()
        .iter()
        .take(10)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    cache.join(CACHE_DIR).join(key)
}

/// The timing of the sound of `source`, whose chunks go in `dir`, probed once.
async fn timing(source: &Path, dir: &Path) -> Result<Option<AudioTiming>, String> {
    let known = TIMINGS
        .lock()
        .ok()
        .and_then(|timings| timings.get(dir).copied());
    if let Some(timing) = known {
        return Ok(timing);
    }
    let timing = probe_audio_timing(source).await?;
    if let Ok(mut timings) = TIMINGS.lock() {
        timings.insert(dir.to_path_buf(), timing);
    }
    Ok(timing)
}

/// The chunk `index` of `file`, cut now unless it was before. `None` when
/// the file is not there or has no sound.
pub async fn chunk_path(
    output: &Path,
    cache: &Path,
    file: &str,
    index: u32,
) -> Result<Option<PathBuf>, String> {
    if index > MAX_CHUNK {
        return Err("音频块超出范围".to_string());
    }
    let source = join_relative(output, file).ok_or("无效的文件路径")?;
    let Ok(metadata) = tokio::fs::metadata(&source).await else {
        return Ok(None);
    };
    if !metadata.is_file() {
        return Ok(None);
    }
    let dir = chunk_dir(cache, file, &metadata);
    let path = dir.join(format!("{index}.flac"));
    if tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return Ok(Some(path));
    }
    let Some(timing) = timing(&source, &dir).await? else {
        return Ok(None);
    };
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("创建音频缓存目录失败: {e}"))?;
    // Cut beside it and moved in whole, so a chunk read while being cut is
    // never half there.
    let part = tempfile::Builder::new()
        .suffix(".flac")
        .tempfile_in(&dir)
        .map_err(|e| format!("创建音频缓存文件失败: {e}"))?
        .into_temp_path();
    extract_preview_audio(
        &source,
        timing,
        f64::from(index) * CHUNK_SECONDS,
        CHUNK_SECONDS,
        PRE_ROLL_SECONDS,
        &part,
    )
    .await?;
    store_chunk(part, &path)?;
    prune_now_and_then(cache);
    Ok(Some(path))
}

/// Move a chunk just cut to `path`, unless one is there already: a chunk is
/// only ever moved in whole, so one cut at the same time, as two editors
/// playing the same file do, is the same. Never replacing it also keeps
/// Windows from failing where it cannot replace a file.
fn store_chunk(part: tempfile::TempPath, path: &Path) -> Result<(), String> {
    match part.persist_noclobber(path) {
        Ok(()) => Ok(()),
        Err(_) if path.is_file() => Ok(()),
        Err(error) => Err(format!("保存音频块失败: {error}")),
    }
}

/// Now and then, in the background, remove the chunks of files not cut
/// again for [`KEEP_FOR`].
fn prune_now_and_then(cache: &Path) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| time.as_secs());
    let last = LAST_PRUNE.load(Ordering::Relaxed);
    if now.saturating_sub(last) < PRUNE_EVERY_SECS
        || LAST_PRUNE
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
    {
        return;
    }
    let root = cache.join(CACHE_DIR);
    tokio::spawn(async move {
        let Ok(mut entries) = tokio::fs::read_dir(&root).await else {
            return;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let stale = entry
                .metadata()
                .await
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > KEEP_FOR);
            if stale {
                let _ = tokio::fs::remove_dir_all(entry.path()).await;
            }
        }
    });
}

/// The response to `GET /preview_audio?file=…&chunk=…`.
pub async fn respond(output: &Path, cache: &Path, query: ChunkQuery) -> Response {
    match chunk_path(output, cache, &query.file, query.chunk).await {
        Ok(Some(path)) => match tokio::fs::read(&path).await {
            Ok(bytes) => (
                [
                    (header::CONTENT_TYPE, "audio/flac"),
                    (header::CACHE_CONTROL, "no-cache"),
                ],
                bytes,
            )
                .into_response(),
            Err(_) => StatusCode::NOT_FOUND.into_response(),
        },
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            log::warn!(
                "Failed to cut preview audio of {} at chunk {}: {error}",
                query.file,
                query.chunk
            );
            (StatusCode::UNPROCESSABLE_ENTITY, error).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_pcm(path: &Path, frames: usize) {
        let mut writer = hound::WavWriter::create(
            path,
            hound::WavSpec {
                channels: 2,
                sample_rate: 48_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for index in 0..frames {
            writer
                .write_sample((12_000.0 * (index as f64 * 0.031).sin()) as i16)
                .unwrap();
            writer
                .write_sample((8_000.0 * (index as f64 * 0.057).sin()) as i16)
                .unwrap();
        }
        writer.finalize().unwrap();
    }

    fn flac_samples(path: &Path) -> u64 {
        let data = std::fs::read(path).unwrap();
        assert_eq!(&data[..4], b"fLaC");
        let format = u64::from_be_bytes(data[18..26].try_into().unwrap());
        assert_eq!(format >> 44, 48_000);
        assert_eq!(((format >> 41) & 7) + 1, 2);
        assert_eq!(((format >> 36) & 31) + 1, 16);
        format & ((1 << 36) - 1)
    }

    #[tokio::test]
    async fn native_preview_cache_keeps_complete_chunks_and_concurrent_requests_without_a_cli() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("output");
        let cache = directory.path().join("cache");
        std::fs::create_dir_all(&output).unwrap();
        let source = output.join("声音 样本.wav");
        write_pcm(&source, 480_101);
        let (first, concurrent) = tokio::join!(
            chunk_path(&output, &cache, "声音 样本.wav", 0),
            chunk_path(&output, &cache, "声音 样本.wav", 0)
        );
        let first = first.unwrap().unwrap();
        assert_eq!(concurrent.unwrap(), Some(first.clone()));
        assert_eq!(flac_samples(&first), 480_000);
        let data = std::fs::read(&first).unwrap();
        assert_eq!(&data[8..12], &[16, 0, 16, 0]);
        let modified = std::fs::metadata(&first).unwrap().modified().unwrap();
        assert_eq!(
            chunk_path(&output, &cache, "声音 样本.wav", 0)
                .await
                .unwrap(),
            Some(first.clone())
        );
        assert_eq!(
            std::fs::metadata(&first).unwrap().modified().unwrap(),
            modified
        );
        assert_eq!(std::fs::read(&first).unwrap(), data);
        let last = chunk_path(&output, &cache, "声音 样本.wav", 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(flac_samples(&last), 101);
        let eof = chunk_path(&output, &cache, "声音 样本.wav", 2)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(flac_samples(&eof), 0);
        assert_eq!(
            std::fs::read_dir(first.parent().unwrap()).unwrap().count(),
            3
        );
        // A changed source gets fresh timing and chunks under a new key.
        write_pcm(&source, 48_017);
        let changed = chunk_path(&output, &cache, "声音 样本.wav", 0)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(changed.parent(), first.parent());
        assert_eq!(flac_samples(&changed), 48_017);
    }

    #[tokio::test]
    async fn native_preview_cache_does_not_create_chunks_for_video_without_audio() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("output");
        let cache = directory.path().join("cache");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/video/test.mp4"),
            output.join("silent.mp4"),
        )
        .unwrap();
        assert_eq!(
            chunk_path(&output, &cache, "silent.mp4", 0).await.unwrap(),
            None
        );
        assert!(!cache.exists());
    }

    #[tokio::test]
    async fn chunks_are_cut_only_from_files_inside_the_output_dir() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("output");
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(dir.path().join("secret.mp4"), "secret").unwrap();
        assert!(chunk_path(&output, &cache, "../secret.mp4", 0)
            .await
            .is_err());
        assert!(chunk_path(&output, &cache, "/etc/passwd", 0).await.is_err());
        assert_eq!(
            chunk_path(&output, &cache, "missing.mp4", 0).await,
            Ok(None)
        );
        assert!(chunk_path(&output, &cache, "missing.mp4", MAX_CHUNK + 1)
            .await
            .is_err());
    }

    #[test]
    fn a_chunk_cut_twice_at_once_keeps_the_first_stored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("0.flac");
        let cut = |content: &str| {
            let part = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
            std::fs::write(part.path(), content).unwrap();
            part.into_temp_path()
        };
        let (first, second) = (cut("first"), cut("second"));
        store_chunk(first, &path).unwrap();
        store_chunk(second, &path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        // Only the chunk is left, not the copy that lost.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_file_changed_under_the_same_name_gets_other_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.mp4");
        std::fs::write(&file, "one").unwrap();
        let before = chunk_dir(dir.path(), "a.mp4", &std::fs::metadata(&file).unwrap());
        assert_eq!(
            before,
            chunk_dir(dir.path(), "a.mp4", &std::fs::metadata(&file).unwrap())
        );
        assert_ne!(
            before,
            chunk_dir(dir.path(), "b.mp4", &std::fs::metadata(&file).unwrap())
        );
        std::fs::write(&file, "longer").unwrap();
        assert_ne!(
            before,
            chunk_dir(dir.path(), "a.mp4", &std::fs::metadata(&file).unwrap())
        );
    }
}
