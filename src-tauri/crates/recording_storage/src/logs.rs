//! Object-only append log protocol. `.recording-logs` is reserved storage metadata.
//!
//! A legacy base object is never rewritten. Each append writes chunks of at most
//! 64 KiB, then atomically replaces an eight-byte committed chunk count. It costs
//! one bounded metadata GET, ceil(bytes / 64 KiB) PUTs, and one metadata PUT.
//! Empty appends do no writes. Opening an existing log costs one metadata GET.
//! Reads use one base GET, one metadata GET and one GET per committed chunk.
//! Read memory is proportional to the complete log, by the public API contract.
//!
//! Only one writer may use a log at a time. Failed/unacknowledged appends may have
//! committed (as with object writes); uncommitted chunks are ignored and reused
//! on retry. No pending data is held until flush/drop. Cleanup removes metadata
//! and orphan chunks along with the recording. Missing committed chunks and
//! malformed metadata are errors, never silently treated as an empty log.
use std::io;

use crate::{validate_key, StorageBackend};

pub const LOG_CHUNK_BYTES: usize = 64 * 1024;

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
    let mut next = count(backend, key).await?.ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "log must be opened before append")
    })?;
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
    for index in 0..count.unwrap_or(0) {
        bytes.extend_from_slice(
            &backend
                .read_bounded(&format!("{directory}/{index:020}"), LOG_CHUNK_BYTES)
                .await
                .map_err(|error| {
                    if error.kind() == io::ErrorKind::NotFound {
                        io::Error::new(io::ErrorKind::InvalidData, "committed log chunk is missing")
                    } else {
                        error
                    }
                })?,
        );
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
