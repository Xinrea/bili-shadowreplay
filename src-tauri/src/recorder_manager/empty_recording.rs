//! Conservative proof for automatic cleanup, not explicit user deletion.
use recording_storage::{RecordingStorage, LOG_METADATA_DIR};
use std::io;

pub(super) async fn confirmed_empty(storage: &RecordingStorage) -> io::Result<bool> {
    // Remote logs create a nonempty head even before the first event. Ask the
    // protocol owner to verify it rather than guessing from its physical size.
    let empty_event_head = storage.empty_log_head("events.jsonl").await?;
    let mut cursor = None;
    let mut empty_playlist = false;
    loop {
        let page = storage.list("", cursor.as_deref(), 1000).await?;
        for key in page.keys {
            match key.as_str() {
                // These are created during setup even before any media arrives.
                "cover.jpg" | ".sequence" => {}
                key if empty_event_head.as_deref() == Some(key) => {}
                // Preserve every other log metadata/chunk object, even zero-byte
                // orphan chunks or a different log whose head was not verified.
                key if key.split('/').any(|part| part == LOG_METADATA_DIR) => return Ok(false),
                "playlist.m3u8" => {
                    let bytes = storage.read_bounded(&key, 64 * 1024 * 1024).await?;
                    let (remaining, playlist) =
                        m3u8_rs::parse_media_playlist(&bytes).map_err(|_| {
                            io::Error::other("Cannot verify an invalid recording playlist")
                        })?;
                    if !remaining.is_empty() || !playlist.segments.is_empty() {
                        return Ok(false);
                    }
                    empty_playlist = true;
                }
                // Keep orphan media, legacy entries, nonempty event logs and
                // unknown assets too. Neither a missing playlist nor failed
                // progress accounting proves these bytes are disposable.
                _ if storage.size(&key).await? != 0 => return Ok(false),
                _ => {}
            }
        }
        match page.next_cursor {
            Some(next) if cursor.as_ref().is_none_or(|previous| next > *previous) => {
                cursor = Some(next);
            }
            Some(_) => return Err(io::Error::other("Recording listing did not progress")),
            // A missing namespace (e.g. a disconnected disk) can list as empty.
            // Require a readable empty manifest, not just an absence of objects.
            None => return Ok(empty_playlist),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn checks_every_page_before_confirming_an_empty_recording() {
        let directory = tempfile::tempdir().unwrap();
        let storage = RecordingStorage::local(directory.path());
        assert!(!confirmed_empty(&storage).await.unwrap());
        storage
            .write("playlist.m3u8", b"#EXTM3U\n#EXT-X-TARGETDURATION:4\n")
            .await
            .unwrap();
        assert!(confirmed_empty(&storage).await.unwrap());
        for index in 0..1001 {
            std::fs::write(directory.path().join(format!("{index:04}.empty")), b"").unwrap();
        }
        storage
            .write("z-media.ts", b"recoverable media")
            .await
            .unwrap();
        assert!(!confirmed_empty(&storage).await.unwrap());
        storage.remove("z-media.ts").await.unwrap();
        assert!(confirmed_empty(&storage).await.unwrap());
    }
}
