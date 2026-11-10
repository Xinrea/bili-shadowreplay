//! Object-only append log protocol. `.recording-logs` is reserved storage metadata.
//!
//! A legacy base object is never rewritten. Each append writes chunks of at most
//! 64 KiB, then atomically replaces an eight-byte committed chunk count. It costs
//! one bounded metadata GET, ceil(bytes / 64 KiB) PUTs, and one metadata PUT.
//! Empty appends do no writes. Opening an existing log costs one metadata GET.
//! Reads use one base GET, one metadata GET and one GET per committed chunk.
//! Chunk reads are bounded-concurrent and assembled in commit order.
//! Read memory is proportional to the complete log, by the public API contract.
//!
//! Only one writer may use a log at a time. Failed/unacknowledged appends may have
//! committed (as with object writes); uncommitted chunks are ignored and reused
//! on retry. No pending data is held until flush/drop. Cleanup removes metadata
//! and orphan chunks along with the recording. Missing committed chunks and
//! malformed metadata are errors, never silently treated as an empty log.
use futures::{stream, StreamExt, TryStreamExt};
use std::io;

use crate::{validate_key, StorageBackend};

pub const LOG_CHUNK_BYTES: usize = 64 * 1024;
const LOG_READ_CONCURRENCY: usize = 8;

fn directory(key: &str) -> io::Result<String> {
    validate_key(key)?;
    let (parent, name) = key
        .rsplit_once('/')
        .map_or(("", key), |(parent, name)| (parent, name));
    // Hex encoding is injective and splitting bounds each path component even
    // for long/nested object keys. The terminator cannot collide with hex data.
    let encoded: String = name
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let components = encoded
        .as_bytes()
        .chunks(120)
        .map(|part| std::str::from_utf8(part).expect("hex is ASCII"))
        .collect::<Vec<_>>()
        .join("/");
    let directory = format!(".recording-logs/{components}/log");
    Ok(if parent.is_empty() {
        directory
    } else {
        format!("{parent}/{directory}")
    })
}

async fn optional_read<B: StorageBackend + ?Sized>(
    backend: &B,
    key: &str,
) -> io::Result<Option<Vec<u8>>> {
    match backend.read(key).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) async fn count<B: StorageBackend + ?Sized>(
    backend: &B,
    key: &str,
) -> io::Result<Option<u64>> {
    let head = format!("{}/head", directory(key)?);
    let bytes = match backend.read_bounded(&head, 8).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid log chunk count"))?;
    Ok(Some(u64::from_le_bytes(bytes)))
}

pub(crate) async fn open<B: StorageBackend + ?Sized>(backend: &B, key: &str) -> io::Result<()> {
    if count(backend, key).await?.is_none() {
        backend
            .write(&format!("{}/head", directory(key)?), &0u64.to_le_bytes())
            .await?;
    }
    Ok(())
}

pub(crate) async fn append<B: StorageBackend + ?Sized>(
    backend: &B,
    key: &str,
    bytes: &[u8],
) -> io::Result<()> {
    let directory = directory(key)?;
    let next = count(backend, key).await?.ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "log must be opened before append")
    })?;
    publish(backend, &directory, bytes, next).await
}

pub(crate) async fn append_at<B: StorageBackend + ?Sized>(
    backend: &B,
    key: &str,
    bytes: &[u8],
    start: u64,
) -> io::Result<()> {
    let directory = directory(key)?;
    let end = start
        .checked_add(bytes.len().div_ceil(LOG_CHUNK_BYTES) as u64)
        .ok_or_else(|| io::Error::other("log chunk count overflow"))?;
    let current = count(backend, key)
        .await?
        .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    if current == end {
        // The head PUT may have succeeded despite a lost response. Verify only
        // this batch, never scan the history or append at the advanced head.
        for (index, expected) in bytes.chunks(LOG_CHUNK_BYTES).enumerate() {
            let key = format!("{directory}/{:020}", start + index as u64);
            if backend.read_bounded(&key, LOG_CHUNK_BYTES).await? != expected {
                return Err(io::Error::other("committed log batch does not match retry"));
            }
        }
        return Ok(());
    }
    if current != start {
        return Err(io::Error::other("log position changed during append"));
    }
    // Reuse the original indices even after an unacknowledged publication.
    publish(backend, &directory, bytes, start).await
}

async fn publish<B: StorageBackend + ?Sized>(
    backend: &B,
    directory: &str,
    bytes: &[u8],
    mut next: u64,
) -> io::Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    for chunk in bytes.chunks(LOG_CHUNK_BYTES) {
        let following = next
            .checked_add(1)
            .ok_or_else(|| io::Error::other("log chunk count overflow"))?;
        backend
            .write(&format!("{directory}/{next:020}"), chunk)
            .await?;
        next = following;
    }
    backend
        .write(&format!("{directory}/head"), &next.to_le_bytes())
        .await
}

pub(crate) async fn read<B: StorageBackend + ?Sized>(
    backend: &B,
    key: &str,
) -> io::Result<Vec<u8>> {
    let directory = directory(key)?;
    let base = optional_read(backend, key).await?;
    let count = count(backend, key).await?;
    if base.is_none() && count.is_none() {
        return Err(io::ErrorKind::NotFound.into());
    }
    let mut bytes = base.unwrap_or_default();
    // The count is a fixed committed snapshot. Chunks are immutable during
    // normal appends, so fetch concurrently without reading uncommitted suffixes.
    // buffered (not buffer_unordered) preserves byte/JSONL order, even if a
    // record crosses a chunk boundary and requests complete out of order.
    let mut chunks = stream::iter(0..count.unwrap_or(0))
        .map(|index| {
            let directory = &directory;
            async move {
                backend
                    .read_bounded(&format!("{directory}/{index:020}"), LOG_CHUNK_BYTES)
                    .await
                    .map_err(|error| {
                        if error.kind() == io::ErrorKind::NotFound {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "committed log chunk is missing",
                            )
                        } else {
                            error
                        }
                    })
            }
        })
        .buffered(LOG_READ_CONCURRENCY);
    while let Some(chunk) = chunks.try_next().await? {
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(crate) async fn exists<B: StorageBackend + ?Sized>(backend: &B, key: &str) -> io::Result<bool> {
    validate_key(key)?;
    let base_exists = match backend.stat(key).await {
        Ok(_) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    Ok(count(backend, key).await?.is_some() || base_exists)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ObjectMetadata, ObjectPage};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct Reader {
        barrier: tokio::sync::Barrier,
        active: AtomicUsize,
        peak: AtomicUsize,
        missing: Option<u64>,
    }

    struct Reading<'a>(&'a AtomicUsize);
    impl Drop for Reading<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl StorageBackend for Reader {
        async fn read(&self, key: &str) -> io::Result<Vec<u8>> {
            assert_eq!(key, "events.jsonl");
            Ok(b"base|".to_vec())
        }
        async fn read_bounded(&self, key: &str, max: usize) -> io::Result<Vec<u8>> {
            if key.ends_with("/head") {
                assert_eq!(max, 8);
                return Ok((2 * LOG_READ_CONCURRENCY as u64).to_le_bytes().to_vec());
            }
            assert_eq!(max, LOG_CHUNK_BYTES);
            let index: u64 = key.rsplit('/').next().unwrap().parse().unwrap();
            assert!(
                index < 2 * LOG_READ_CONCURRENCY as u64,
                "read past committed head"
            );
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            let _reading = Reading(&self.active);
            self.peak.fetch_max(active, Ordering::SeqCst);
            // Serial loading would deadlock here; a bounded batch must reach it.
            self.barrier.wait().await;
            if self.missing == Some(index) {
                return Err(io::ErrorKind::NotFound.into());
            }
            tokio::time::sleep(Duration::from_millis(2 * (8 - index % 8))).await;
            Ok(format!("{index:02}|").into_bytes())
        }
        async fn read_range(&self, _: &str, _: std::ops::Range<u64>) -> io::Result<Vec<u8>> {
            unreachable!()
        }
        async fn write(&self, _: &str, _: &[u8]) -> io::Result<()> {
            panic!("history reads must not write")
        }
        async fn remove(&self, _: &str) -> io::Result<()> {
            unreachable!()
        }
        async fn stat(&self, _: &str) -> io::Result<ObjectMetadata> {
            unreachable!()
        }
        async fn list(&self, _: &str, _: Option<&str>, _: usize) -> io::Result<ObjectPage> {
            unreachable!()
        }
    }

    fn reader(missing: Option<u64>) -> Reader {
        Reader {
            barrier: tokio::sync::Barrier::new(LOG_READ_CONCURRENCY),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            missing,
        }
    }

    #[tokio::test]
    async fn history_reads_are_bounded_parallel_but_keep_byte_order() {
        let backend = reader(None);
        let bytes = tokio::time::timeout(Duration::from_secs(2), read(&backend, "events.jsonl"))
            .await
            .unwrap()
            .unwrap();
        let expected = format!(
            "base|{}",
            (0..16).map(|i| format!("{i:02}|")).collect::<String>()
        );
        assert_eq!(bytes, expected.as_bytes());
        assert_eq!(backend.peak.load(Ordering::SeqCst), LOG_READ_CONCURRENCY);
        assert_eq!(backend.active.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn missing_history_chunk_is_not_published_as_partial_success() {
        let backend = reader(Some(2));
        let error = tokio::time::timeout(Duration::from_secs(2), read(&backend, "events.jsonl"))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(backend.active.load(Ordering::SeqCst), 0);
        assert!(backend.peak.load(Ordering::SeqCst) <= LOG_READ_CONCURRENCY);
    }
}
