use std::{sync::Arc, time::Duration};

use danmu_stream::{danmu_stream::DanmuStream, provider::ProviderType, DanmuMessageType};
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let room_id = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "599934".to_string());
    let cookie = "";
    let stream = Arc::new(DanmuStream::new(ProviderType::Huya, cookie, &room_id).await?);

    log::info!("Start to receive huya danmu for room {room_id}");

    let stream_clone = stream.clone();
    tokio::spawn(async move {
        loop {
            if let Ok(Some(msg)) = stream_clone.recv().await {
                match msg {
                    DanmuMessageType::Event(event) => {
                        log::info!("Received event: {:?}", event.event_type);
                    }
                    DanmuMessageType::DanmuMessage(danmu) => {
                        log::info!("[{}] {}", danmu.user_name, danmu.message);
                    }
                }
            } else {
                log::info!("Channel closed");
                break;
            }
        }
    });

    let _ = stream.start().await;

    sleep(Duration::from_secs(60)).await;

    stream.stop().await?;

    Ok(())
}
