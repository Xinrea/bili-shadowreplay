use std::{sync::Arc, time::Duration};

use danmu_stream::{danmu_stream::DanmuStream, provider::ProviderType, DanmuMessageType};
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    env_logger::init();
    // Replace these with actual values
    let room_id = "7514298567821937427"; // Replace with actual Douyin room_id. When live starts, the room_id will be generated, so it's more like a live_id.
    let cookie = "your_cookie";
    let stream = Arc::new(DanmuStream::new(ProviderType::Douyin, cookie, room_id).await?);

    log::info!("Start to receive danmu messages");

    let _ = stream.start().await;

    let stream_clone = stream.clone();
    tokio::spawn(async move {
        loop {
            if let Ok(Some(msg)) = stream_clone.recv().await {
                match msg {
                    DanmuMessageType::DanmuMessage(danmu) => {
                        log::info!("Received danmu message: {:?}", danmu.message);
                    }
                }
            } else {
                log::info!("Channel closed");
                break;
            }
        }
    });

    sleep(Duration::from_secs(10)).await;

    stream.stop().await?;

    Ok(())
}
