use std::{sync::Arc, time::Duration};

use danmu_stream::{danmu_stream::DanmuStream, provider::ProviderType, DanmuMessageType};
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    env_logger::init();
    // Replace these with actual values
    let room_id = "768756";
    let cookie = "";
    let stream = Arc::new(DanmuStream::new(ProviderType::BiliBili, cookie, room_id).await?);

    log::info!("Start to receive danmu messages: {}", cookie);

    let stream_clone = stream.clone();
    tokio::spawn(async move {
        loop {
            log::info!("Waitting for message");
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

    let _ = stream.start().await;

    sleep(Duration::from_secs(10)).await;

    stream.stop().await?;

    Ok(())
}
