use m3u8_rs::{MediaPlaylist, MediaPlaylistType, MediaSegment};
use std::path::PathBuf;

use crate::errors::RecorderError;

pub struct HlsPlaylist {
    pub playlist: MediaPlaylist,
    pub file_path: PathBuf,
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
        let mut playlist = if file_path.exists() {
            let bytes = tokio::fs::read(&file_path)
                .await
                .map_err(RecorderError::IoError)?;
            let (_, playlist) = m3u8_rs::parse_media_playlist(&bytes).map_err(|_| {
                RecorderError::M3u8ParseFailed {
                    content: playlist_content_preview(&bytes),
                }
            })?;

            playlist
        } else {
            MediaPlaylist::default()
        };
        normalize_archive_playlist(&mut playlist);
        Ok(Self {
            playlist,
            file_path,
        })
    }

    pub async fn last_segment(&self) -> Option<&MediaSegment> {
        self.playlist.segments.last()
    }

    pub async fn append_last_segment(
        &mut self,
        segment: MediaSegment,
    ) -> Result<(), RecorderError> {
        let Some(last) = self.playlist.segments.last_mut() else {
            self.add_segment(segment).await?;
            return Ok(());
        };

        let new_duration = last.duration + segment.duration;
        last.duration = new_duration;
        self.playlist.target_duration = self
            .playlist
            .target_duration
            .max(new_duration.ceil() as u64);
        self.flush().await?;

        Ok(())
    }

    pub async fn add_segment(&mut self, segment: MediaSegment) -> Result<(), RecorderError> {
        self.playlist.target_duration = self
            .playlist
            .target_duration
            .max(segment.duration.ceil() as u64);
        self.playlist.segments.push(segment);
        self.flush().await?;
        Ok(())
    }

    pub async fn flush(&self) -> Result<(), RecorderError> {
        // Create an in-memory buffer to serialize the playlist into.
        // `Vec<u8>` implements `std::io::Write`, which `m3u8_rs::MediaPlaylist::write_to` expects.
        let mut buffer = Vec::new();

        // Serialize the playlist into the buffer.
        self.playlist
            .write_to(&mut buffer)
            .map_err(RecorderError::IoError)?;

        // Write the buffer to the file
        tokio::fs::write(&self.file_path, buffer)
            .await
            .map_err(RecorderError::IoError)?;

        Ok(())
    }

    pub async fn close(&mut self) -> Result<(), RecorderError> {
        self.playlist.end_list = true;
        self.playlist.playlist_type = Some(MediaPlaylistType::Vod);
        self.flush().await?;
        Ok(())
    }

    /// Reopen a previously closed playlist when a live recording resumes in
    /// the same archive directory after a temporary source timeout.
    pub(crate) async fn reopen(&mut self) -> Result<(), RecorderError> {
        self.playlist.end_list = false;
        self.playlist.playlist_type = Some(MediaPlaylistType::Event);
        // Persist normalized headers even for a legacy playlist that was never
        // closed, so a newly opened preview immediately sees EVENT semantics.
        self.flush().await
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
