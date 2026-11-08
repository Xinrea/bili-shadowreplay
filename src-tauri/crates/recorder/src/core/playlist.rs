use m3u8_rs::{MediaPlaylist, MediaPlaylistType, MediaSegment};
use recording_storage::RecordingStorage;
use std::path::PathBuf;

use crate::errors::RecorderError;

pub struct HlsPlaylist {
    pub playlist: MediaPlaylist,
    pub file_path: PathBuf,
    storage: RecordingStorage,
    key: String,
}

/// Archives retain every recorded segment. Advertising a sliding LIVE window
/// lets players evict segments that are still needed after a slow download.
/// Also repair older archives whose target duration was never initialized.
pub fn normalize_archive_playlist(playlist: &mut MediaPlaylist) {
    playlist.playlist_type = Some(if playlist.end_list {
        MediaPlaylistType::Vod
    } else {
        MediaPlaylistType::Event
    });
    playlist.target_duration = playlist
        .segments
        .iter()
        .fold(playlist.target_duration.max(1), |target, segment| {
            target.max(segment.duration.ceil() as u64)
        });
}

impl HlsPlaylist {
    pub async fn new(file_path: PathBuf) -> Result<Self, RecorderError> {
        let key = file_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "invalid playlist filename",
                )
            })?;
        let root = file_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        Self::from_storage(RecordingStorage::local(root), key).await
    }

    pub async fn from_storage(storage: RecordingStorage, key: &str) -> Result<Self, RecorderError> {
        let file_path = storage.workspace().path(key)?;
        let mut playlist = match storage.read(key).await {
            Ok(bytes) => {
                let (_, playlist) = m3u8_rs::parse_media_playlist(&bytes).map_err(|_| {
                    RecorderError::M3u8ParseFailed {
                        content: playlist_content_preview(&bytes),
                    }
                })?;

                playlist
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => MediaPlaylist::default(),
            Err(error) => return Err(RecorderError::IoError(error)),
        };
        normalize_archive_playlist(&mut playlist);
        Ok(Self {
            playlist,
            file_path,
            storage,
            key: key.to_owned(),
        })
    }

    pub async fn last_segment(&self) -> Option<&MediaSegment> {
        self.playlist.segments.last()
    }

    pub async fn append_last_segment(
        &mut self,
        segment: MediaSegment,
    ) -> Result<(), RecorderError> {
        let mut next = self.playlist.clone();
        let Some(last) = next.segments.last_mut() else {
            self.add_segment(segment).await?;
            return Ok(());
        };

        let new_duration = last.duration + segment.duration;
        last.duration = new_duration;
        next.target_duration = next.target_duration.max(new_duration.ceil() as u64);
        self.replace(next).await
    }

    pub async fn add_segment(&mut self, segment: MediaSegment) -> Result<(), RecorderError> {
        let mut next = self.playlist.clone();
        next.target_duration = next.target_duration.max(segment.duration.ceil() as u64);
        next.segments.push(segment);
        self.replace(next).await
    }

    pub async fn flush(&self) -> Result<(), RecorderError> {
        self.persist(&self.playlist).await
    }

    async fn replace(&mut self, next: MediaPlaylist) -> Result<(), RecorderError> {
        // A failed publication must not advance the in-memory manifest either.
        self.persist(&next).await?;
        self.playlist = next;
        Ok(())
    }

    async fn persist(&self, playlist: &MediaPlaylist) -> Result<(), RecorderError> {
        // Create an in-memory buffer to serialize the playlist into.
        // `Vec<u8>` implements `std::io::Write`, which `m3u8_rs::MediaPlaylist::write_to` expects.
        let mut buffer = Vec::new();

        // Serialize the playlist into the buffer.
        playlist
            .write_to(&mut buffer)
            .map_err(RecorderError::IoError)?;

        // Publish a complete playlist; readers never observe a truncated rewrite.
        self.storage
            .write(&self.key, &buffer)
            .await
            .map_err(RecorderError::IoError)?;

        Ok(())
    }

    pub async fn close(&mut self) -> Result<(), RecorderError> {
        let mut next = self.playlist.clone();
        next.end_list = true;
        next.playlist_type = Some(MediaPlaylistType::Vod);
        self.replace(next).await
    }

    /// Reopen a previously closed playlist when a live recording resumes in
    /// the same archive directory after a temporary source timeout.
    pub(crate) async fn reopen(&mut self) -> Result<(), RecorderError> {
        let mut next = self.playlist.clone();
        next.end_list = false;
        next.playlist_type = Some(MediaPlaylistType::Event);
        // Persist normalized headers even for a legacy playlist that was never
        // closed, so a newly opened preview immediately sees EVENT semantics.
        self.replace(next).await
    }

    pub async fn is_empty(&self) -> bool {
        self.playlist.segments.is_empty()
    }
}

pub(super) fn playlist_content_preview(bytes: &[u8]) -> String {
    const MAX_PREVIEW_CHARS: usize = 256;
    let content = String::from_utf8_lossy(bytes);
    let mut chars = content.chars();
    let mut preview: String = chars.by_ref().take(MAX_PREVIEW_CHARS).collect();
    if chars.next().is_some() {
        preview.push_str("...");
    }
    preview.escape_debug().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use recording_storage::StorageBackend;

    struct RejectWrites(recording_storage::LocalBackend);

    #[async_trait::async_trait]
    impl StorageBackend for RejectWrites {
        async fn read_range(
            &self,
            key: &str,
            range: std::ops::Range<u64>,
        ) -> std::io::Result<Vec<u8>> {
            self.0.read_range(key, range).await
        }
        async fn list(
            &self,
            prefix: &str,
            cursor: Option<&str>,
            limit: usize,
        ) -> std::io::Result<recording_storage::ObjectPage> {
            self.0.list(prefix, cursor, limit).await
        }
        async fn read(&self, key: &str) -> std::io::Result<Vec<u8>> {
            self.0.read(key).await
        }

        async fn read_bounded(&self, key: &str, max_bytes: usize) -> std::io::Result<Vec<u8>> {
            self.0.read_bounded(key, max_bytes).await
        }

        async fn write(&self, _: &str, _: &[u8]) -> std::io::Result<()> {
            Err(std::io::ErrorKind::PermissionDenied.into())
        }

        async fn remove(&self, key: &str) -> std::io::Result<()> {
            self.0.remove(key).await
        }

        async fn stat(&self, key: &str) -> std::io::Result<recording_storage::ObjectMetadata> {
            self.0.stat(key).await
        }
    }

    #[tokio::test]
    async fn failed_publication_keeps_persisted_and_in_memory_playlist() {
        let dir = tempfile::tempdir().unwrap();
        let local = RecordingStorage::local(dir.path());
        let mut original = HlsPlaylist::from_storage(local.clone(), "playlist.m3u8")
            .await
            .unwrap();
        original
            .add_segment(MediaSegment {
                uri: "0.ts".into(),
                duration: 4.0,
                ..Default::default()
            })
            .await
            .unwrap();
        let before = local.read("playlist.m3u8").await.unwrap();
        let storage = RecordingStorage::with_backend(
            std::sync::Arc::new(RejectWrites(recording_storage::LocalBackend::new(
                dir.path(),
            ))),
            recording_storage::LocalWorkspace::new(dir.path()),
        );
        let mut playlist = HlsPlaylist::from_storage(storage, "playlist.m3u8")
            .await
            .unwrap();
        let segment = MediaSegment {
            uri: "1.ts".into(),
            duration: 6.0,
            ..Default::default()
        };
        assert!(playlist.add_segment(segment.clone()).await.is_err());
        assert!(playlist.append_last_segment(segment).await.is_err());
        assert!(playlist.close().await.is_err());
        assert_eq!(playlist.playlist, original.playlist);
        assert_eq!(local.read("playlist.m3u8").await.unwrap(), before);
    }

    #[tokio::test]
    async fn new_returns_parse_error_for_corrupt_playlist() {
        let path = std::env::temp_dir().join(format!(
            "bili-shadowreplay-corrupt-{}.m3u8",
            uuid::Uuid::new_v4()
        ));
        tokio::fs::write(&path, vec![0; 1024]).await.unwrap();

        let result = HlsPlaylist::new(path.clone()).await;

        assert!(matches!(result, Err(RecorderError::M3u8ParseFailed { .. })));
        let _ = tokio::fs::remove_file(path).await;
    }

    #[tokio::test]
    async fn new_starts_empty_when_playlist_does_not_exist() {
        let path = std::env::temp_dir().join(format!(
            "bili-shadowreplay-missing-{}.m3u8",
            uuid::Uuid::new_v4()
        ));

        let playlist = HlsPlaylist::new(path).await.unwrap();

        assert!(playlist.playlist.segments.is_empty());
        assert_eq!(
            playlist.playlist.playlist_type,
            Some(MediaPlaylistType::Event)
        );
        assert_eq!(playlist.playlist.target_duration, 1);
    }

    #[tokio::test]
    async fn target_duration_covers_added_and_appended_fractional_segments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("playlist.m3u8");
        let mut playlist = HlsPlaylist::new(path.clone()).await.unwrap();
        for (duration, target) in [(4.166667, 5), (3.2, 5), (6.066667, 7)] {
            playlist
                .add_segment(MediaSegment {
                    uri: format!("segment-{}.ts", playlist.playlist.segments.len()),
                    duration,
                    ..Default::default()
                })
                .await
                .unwrap();
            assert_eq!(playlist.playlist.target_duration, target);
        }
        playlist
            .append_last_segment(MediaSegment {
                duration: 2.1,
                ..Default::default()
            })
            .await
            .unwrap();

        let bytes = tokio::fs::read(&path).await.unwrap();
        let (_, saved) = m3u8_rs::parse_media_playlist(&bytes).unwrap();
        assert_eq!(saved.playlist_type, Some(MediaPlaylistType::Event));
        assert_eq!(saved.target_duration, 9);
        assert!(!saved.end_list);
        assert_eq!(saved.segments.len(), 3);
        assert!((saved.segments[2].duration - 8.166667).abs() < 0.001);
    }

    #[tokio::test]
    async fn reopening_an_unclosed_legacy_playlist_persists_repaired_headers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("playlist.m3u8");
        tokio::fs::write(
            &path,
            "#EXTM3U\n#EXT-X-TARGETDURATION:0\n#EXTINF:4.166667,\nsegment.ts\n",
        )
        .await
        .unwrap();

        let mut playlist = HlsPlaylist::new(path.clone()).await.unwrap();
        playlist.reopen().await.unwrap();

        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(contents.contains("#EXT-X-PLAYLIST-TYPE:EVENT"));
        assert!(contents.contains("#EXT-X-TARGETDURATION:5"));
        assert!(!contents.contains("#EXT-X-ENDLIST"));
    }

    #[test]
    fn legacy_playback_headers_are_repaired_without_changing_media() {
        for end_list in [false, true] {
            let content = format!(
                "#EXTM3U\n#EXT-X-TARGETDURATION:0\n#EXT-X-MEDIA-SEQUENCE:42\n\
                 #EXT-X-MAP:URI=\"init.mp4\"\n\
                 #EXT-X-PROGRAM-DATE-TIME:2026-10-29T18:42:50+08:00\n\
                 #EXTINF:4.166667,\nsegment.m4s\n{}",
                if end_list { "#EXT-X-ENDLIST\n" } else { "" }
            );
            let (_, mut playlist) = m3u8_rs::parse_media_playlist(content.as_bytes()).unwrap();
            let segments = playlist.segments.clone();
            normalize_archive_playlist(&mut playlist);

            assert_eq!(playlist.end_list, end_list);
            assert_eq!(playlist.media_sequence, 42);
            assert_eq!(playlist.segments, segments);
            assert_eq!(playlist.target_duration, 5);
            assert_eq!(
                playlist.playlist_type,
                Some(if end_list {
                    MediaPlaylistType::Vod
                } else {
                    MediaPlaylistType::Event
                })
            );
            // Repairing a playlist must never shrink a previously advertised target.
            playlist.target_duration = 10;
            normalize_archive_playlist(&mut playlist);
            assert_eq!(playlist.target_duration, 10);
        }
    }

    #[tokio::test]
    async fn reopen_removes_vod_end_marker_for_resumed_recording() {
        let path = std::env::temp_dir().join(format!(
            "bili-shadowreplay-resume-{}.m3u8",
            uuid::Uuid::new_v4()
        ));
        let mut playlist = HlsPlaylist::new(path.clone()).await.unwrap();
        playlist
            .add_segment(MediaSegment {
                uri: "segment_1.ts".to_string(),
                duration: 6.0,
                ..Default::default()
            })
            .await
            .unwrap();
        playlist.close().await.unwrap();
        assert!(playlist.playlist.end_list);

        playlist.reopen().await.unwrap();

        assert!(!playlist.playlist.end_list);
        assert_eq!(
            playlist.playlist.playlist_type,
            Some(MediaPlaylistType::Event)
        );
        let contents = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(!contents.contains("#EXT-X-ENDLIST"));
        let _ = tokio::fs::remove_file(path).await;
    }

    #[test]
    fn content_preview_is_bounded_and_escaped() {
        let preview = playlist_content_preview(&vec![0; 1024]);

        assert!(preview.len() < 2048);
        assert!(preview.ends_with("..."));
        assert!(preview.contains("\\0"));
    }
}
