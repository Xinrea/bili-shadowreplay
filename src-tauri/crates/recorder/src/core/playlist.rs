use m3u8_rs::{MediaPlaylist, MediaPlaylistType, MediaSegment};
use std::path::PathBuf;

use crate::errors::RecorderError;

pub struct HlsPlaylist {
    pub playlist: MediaPlaylist,
    pub file_path: PathBuf,
}

impl HlsPlaylist {
    pub async fn new(file_path: PathBuf) -> Result<Self, RecorderError> {
        if file_path.exists() {
            let bytes = tokio::fs::read(&file_path)
                .await
                .map_err(RecorderError::IoError)?;
            let (_, playlist) = m3u8_rs::parse_media_playlist(&bytes).map_err(|_| {
                RecorderError::M3u8ParseFailed {
                    content: playlist_content_preview(&bytes),
                }
            })?;

            Ok(Self {
                playlist,
                file_path,
            })
        } else {
            Ok(Self {
                playlist: MediaPlaylist::default(),
                file_path,
            })
        }
    }

    pub async fn last_segment(&self) -> Option<&MediaSegment> {
        self.playlist.segments.last()
    }

    pub async fn append_last_segment(
        &mut self,
        segment: MediaSegment,
    ) -> Result<(), RecorderError> {
        if self.is_empty().await {
            self.add_segment(segment).await?;
            return Ok(());
        }

        {
            let last = self.playlist.segments.last_mut().unwrap();
            let new_duration = last.duration + segment.duration;
            last.duration = new_duration;
            self.playlist.target_duration =
                std::cmp::max(self.playlist.target_duration, new_duration as u64);
            self.flush().await?;
        }

        Ok(())
    }

    pub async fn add_segment(&mut self, segment: MediaSegment) -> Result<(), RecorderError> {
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
    }

    #[test]
    fn content_preview_is_bounded_and_escaped() {
        let preview = playlist_content_preview(&vec![0; 1024]);

        assert!(preview.len() < 2048);
        assert!(preview.ends_with("..."));
        assert!(preview.contains("\\0"));
    }
}
