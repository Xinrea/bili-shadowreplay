//! Run with `cargo run -p recording_storage --example local_recording --locked`.
use recording_storage::RecordingStore;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let directory = tempfile::tempdir()?;
    let store = RecordingStore::local(directory.path());
    let recording = store.recording("bilibili", "123", "example-live")?;
    recording.prepare().await?;

    // A completed media object goes directly to the backend. These bytes
    // stand in for a download/muxed segment; they are not playable MPEG-TS.
    recording.write("0.ts", b"example segment").await?;

    recording
        .write(
            "playlist.m3u8",
            b"#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\n0.ts\n",
        )
        .await?;
    recording.write(".sequence", b"1").await?;
    let log = recording.open_log("events.jsonl").await?;
    log.append(b"{\"text\":\"hello\"}\n").await?;
    log.flush().await?;

    assert_eq!(recording.read(".sequence").await?, b"1");
    assert_eq!(recording.size("0.ts").await?, 15);
    assert_eq!(
        recording.read_log("events.jsonl").await?,
        b"{\"text\":\"hello\"}\n"
    );
    println!(
        "Published segment, playlist, checkpoint and append log under {}",
        recording.workspace().root().display()
    );
    // TempDir removes this demonstration archive on exit.
    Ok(())
}
