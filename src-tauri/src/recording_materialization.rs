//! Snapshot archive inputs into an owned workspace for local-only media tools.
//!
//! Keep the original manifest: trimming it would change range offsets and lose
//! inherited initialization maps. Only the selected media dependencies are copied.
use std::collections::BTreeSet;
use std::io;

use m3u8_rs::MediaPlaylist;
use recording_storage::{validate_key, LocalAssets, RecordingStorage};

use crate::ffmpeg::Range;
use crate::recorder_manager::RecorderManagerError;

fn dependency_keys(playlist: &MediaPlaylist, ranges: &[Range]) -> io::Result<Vec<String>> {
    let mut keys = BTreeSet::new();
    let mut active_map = None;
    let mut active_key = None;
    let mut duration = 0.0;
    for segment in &playlist.segments {
        validate_key(&segment.uri)?;
        let map_uri = segment.map.as_ref().map(|map| map.uri.clone()).or_else(|| {
            // Older archives recorded this tag without m3u8-rs parsing it.
            segment
                .unknown_tags
                .iter()
                .find(|tag| tag.tag == "X-MAP")
                .and_then(|tag| tag.rest.as_deref())
                .and_then(|rest| rest.split_once('='))
                .map(|(_, value)| {
                    value
                        .trim()
                        .replace("\\\"", "\"")
                        .trim_matches('"')
                        .to_string()
                })
        });
        if let Some(uri) = map_uri {
            validate_key(&uri)?;
            active_map = Some(uri);
        }
        if let Some(key) = &segment.key {
            if let Some(uri) = &key.uri {
                validate_key(uri)?;
            }
            active_key = key.uri.clone();
        }
        let end = duration + segment.duration as f64;
        // Match ffmpeg::playlist's segment selection, including boundary segments.
        if ranges.is_empty()
            || ranges
                .iter()
                .any(|range| range.is_in(duration) || range.is_in(end))
        {
            keys.insert(segment.uri.clone());
            keys.extend(active_map.iter().cloned());
            keys.extend(active_key.iter().cloned());
        }
        duration = end;
    }
    Ok(keys.into_iter().collect())
}

pub(crate) async fn materialize(
    storage: &RecordingStorage,
    ranges: &[Range],
) -> Result<LocalAssets, RecorderManagerError> {
    let bytes = storage.read("playlist.m3u8").await?;
    let (_, playlist) = m3u8_rs::parse_media_playlist(&bytes).map_err(|_| {
        RecorderManagerError::M3u8ParseFailed {
            content: String::from_utf8_lossy(&bytes).into_owned(),
        }
    })?;
    let keys = dependency_keys(&playlist, ranges)?;
    let assets = storage.materialize(&keys).await?;
    // Do not re-read the manifest after copying: a live recording may advance.
    assets.write("playlist.m3u8", &bytes).await?;
    Ok(assets)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYLIST: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:2,\nfirst.m4s\n#EXTINF:2,\nsecond.m4s\n#EXT-X-MAP:URI=\"next.mp4\"\n#EXTINF:2,\nthird.m4s\n#EXT-X-ENDLIST\n";

    #[test]
    fn range_dependencies_include_inherited_init_without_changing_timeline() {
        let (_, playlist) = m3u8_rs::parse_media_playlist(PLAYLIST.as_bytes()).unwrap();
        assert_eq!(
            dependency_keys(
                &playlist,
                &[Range {
                    start: 2.1,
                    end: 4.1
                }]
            )
            .unwrap(),
            vec!["init.mp4", "next.mp4", "second.m4s", "third.m4s"]
        );
        assert_eq!(
            dependency_keys(&playlist, &[]).unwrap(),
            vec![
                "first.m4s",
                "init.mp4",
                "next.mp4",
                "second.m4s",
                "third.m4s"
            ]
        );
    }

    #[test]
    fn rejects_external_and_traversing_dependencies_even_outside_selected_range() {
        for uri in [
            "https://example.com/video.ts",
            "../video.ts",
            "/video.ts",
            "file:video.ts",
            "dir\\video.ts",
        ] {
            let text = PLAYLIST.replace("first.m4s", uri);
            let (_, playlist) = m3u8_rs::parse_media_playlist(text.as_bytes()).unwrap();
            assert!(dependency_keys(
                &playlist,
                &[Range {
                    start: 2.1,
                    end: 3.9
                }]
            )
            .is_err());
        }
        let text = PLAYLIST.replace("init.mp4", "https://example.com/init.mp4");
        let (_, playlist) = m3u8_rs::parse_media_playlist(text.as_bytes()).unwrap();
        assert!(dependency_keys(&playlist, &[]).is_err());
    }

    #[tokio::test]
    async fn snapshot_copies_only_dependencies_and_owns_their_lifetime() {
        let dir = tempfile::tempdir().unwrap();
        let storage = RecordingStorage::local(dir.path());
        storage
            .write("playlist.m3u8", PLAYLIST.as_bytes())
            .await
            .unwrap();
        storage.write("init.mp4", b"init").await.unwrap();
        storage.write("next.mp4", b"next init").await.unwrap();
        storage.write("second.m4s", b"media").await.unwrap();
        storage.write("third.m4s", b"more media").await.unwrap();
        // Unselected segments intentionally do not exist in storage.
        let assets = materialize(
            &storage,
            &[Range {
                start: 2.1,
                end: 4.1,
            }],
        )
        .await
        .unwrap();
        let playlist_path = assets.path("playlist.m3u8").unwrap();
        assert_eq!(
            assets.read("playlist.m3u8").await.unwrap(),
            PLAYLIST.as_bytes()
        );
        assert_eq!(assets.read("init.mp4").await.unwrap(), b"init");
        assert_eq!(assets.read("second.m4s").await.unwrap(), b"media");
        assert!(!assets.path("first.m4s").unwrap().exists());
        storage.write("playlist.m3u8", b"changed").await.unwrap();
        assert_eq!(
            assets.read("playlist.m3u8").await.unwrap(),
            PLAYLIST.as_bytes()
        );
        drop(assets);
        assert!(!playlist_path.exists());
        assert_eq!(storage.read("second.m4s").await.unwrap(), b"media");
    }
}
