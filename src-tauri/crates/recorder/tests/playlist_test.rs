use recorder::core::playlist::HlsPlaylist;
use std::path::PathBuf;

fn load_fixture(name: &str) -> String {
    let fixture_path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(fixture_path)
        .unwrap_or_else(|_| panic!("Failed to load fixture: {}", name))
}

#[tokio::test]
async fn test_m3u8_parsing_from_file() {
    // Create a temp file with m3u8 content
    let m3u8_content = load_fixture("test_playlist.m3u8");
    let temp_path = std::env::temp_dir().join(format!("test-{}.m3u8", uuid::Uuid::new_v4()));
    tokio::fs::write(&temp_path, m3u8_content).await.unwrap();
    
    let result = HlsPlaylist::new(temp_path.clone()).await;
    assert!(result.is_ok(), "M3U8 parsing should succeed");
    
    let playlist = result.unwrap();
    
    // Check segments
    assert_eq!(playlist.playlist.segments.len(), 3);
    assert_eq!(playlist.playlist.segments[0].uri, "segment-0.ts");
    assert_eq!(playlist.playlist.segments[1].uri, "segment-1.ts");
    assert_eq!(playlist.playlist.segments[2].uri, "segment-2.ts");
    
    // Check duration
    assert_eq!(playlist.playlist.target_duration, 10);
    
    // Clean up
    let _ = tokio::fs::remove_file(temp_path).await;
}

#[tokio::test]
async fn test_m3u8_media_sequence() {
    let m3u8_content = load_fixture("test_playlist.m3u8");
    let temp_path = std::env::temp_dir().join(format!("test-{}.m3u8", uuid::Uuid::new_v4()));
    tokio::fs::write(&temp_path, m3u8_content).await.unwrap();
    
    let playlist = HlsPlaylist::new(temp_path.clone()).await.unwrap();
    
    assert_eq!(playlist.playlist.media_sequence, 0);
    
    // Clean up
    let _ = tokio::fs::remove_file(temp_path).await;
}

#[tokio::test]
async fn test_playlist_empty() {
    let temp_path = std::env::temp_dir().join(format!("test-empty-{}.m3u8", uuid::Uuid::new_v4()));
    
    let playlist = HlsPlaylist::new(temp_path.clone()).await.unwrap();
    
    assert!(playlist.is_empty().await);
    
    // Clean up (file may not exist)
    let _ = tokio::fs::remove_file(temp_path).await;
}
