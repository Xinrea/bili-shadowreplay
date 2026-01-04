use m3u8_rs::{MediaPlaylist, MediaPlaylistType, MediaSegment};
use std::{collections::HashSet, path::PathBuf};

use crate::errors::RecorderError;

pub struct HlsPlaylist {
    pub playlist: MediaPlaylist,
    pub file_path: PathBuf,
    segment_set: HashSet<String>,
}

impl HlsPlaylist {
    pub async fn new(file_path: PathBuf) -> Self {
        if file_path.exists() {
            let bytes = tokio::fs::read(&file_path).await.unwrap();
            let (_, playlist) = m3u8_rs::parse_media_playlist(&bytes).unwrap();
            // create set with all segment path
            let segment_set = playlist
                .segments
                .iter()
                .map(|segment| segment.uri.clone())
                .collect();
            Self {
                playlist,
                file_path,
                segment_set,
            }
        } else {
            Self {
                playlist: MediaPlaylist::default(),
                file_path,
                segment_set: HashSet::new(),
            }
        }
    }

    pub async fn contains_segment(&self, segment_uri: &str) -> bool {
        self.segment_set.contains(segment_uri)
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
        self.segment_set.insert(segment.uri.clone());
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
