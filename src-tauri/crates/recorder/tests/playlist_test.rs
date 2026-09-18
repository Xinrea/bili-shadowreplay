use recorder::core::playlist::HlsPlaylist;

fn load_fixture(name: &str) -> String {
    let fixture_path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(fixture_path)
        .unwrap_or_else(|_| panic!("Failed to load fixture: {}", name))
}

#[tokio::test]
async fn test_m3u8_parsing_from_real_bilibili_playlist() {
    let m3u8_content = load_fixture("test_playlist.m3u8");
    let temp_path = std::env::temp_dir().join(format!("test-{}.m3u8", uuid::Uuid::new_v4()));
    tokio::fs::write(&temp_path, &m3u8_content).await.unwrap();

    let playlist = HlsPlaylist::new(temp_path.clone())
        .await
        .expect("parse real m3u8");
    assert!(
        !playlist.playlist.segments.is_empty(),
        "real Bilibili playlist should contain segments"
    );
    assert!(playlist.playlist.target_duration >= 1);
    assert!(playlist.playlist.media_sequence > 0);
    // Bilibili fmp4 HLS uses .m4s segments
    assert!(
        playlist
            .playlist
            .segments
            .iter()
            .any(|s| s.uri.ends_with(".m4s") || s.uri.ends_with(".ts")),
        "expected media segment URIs, got {:?}",
        playlist
            .playlist
            .segments
            .iter()
            .map(|s| &s.uri)
            .collect::<Vec<_>>()
    );

    let _ = tokio::fs::remove_file(temp_path).await;
}

#[tokio::test]
async fn test_playlist_empty() {
    let temp_path = std::env::temp_dir().join(format!("test-empty-{}.m3u8", uuid::Uuid::new_v4()));

    let playlist = HlsPlaylist::new(temp_path.clone()).await.unwrap();
    assert!(playlist.is_empty().await);

    let _ = tokio::fs::remove_file(temp_path).await;
}
