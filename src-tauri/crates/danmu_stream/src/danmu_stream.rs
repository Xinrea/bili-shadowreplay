use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};

use crate::{
    provider::{new, DanmuProvider, ProviderType},
    DanmuMessageType, DanmuStreamError,
};

#[derive(Clone)]
pub struct DanmuStream {
    pub provider_type: ProviderType,
    pub identifier: String,
    pub room_id: String,
    pub provider: Arc<RwLock<Box<dyn DanmuProvider>>>,
    tx: mpsc::UnboundedSender<DanmuMessageType>,
    rx: Arc<RwLock<mpsc::UnboundedReceiver<DanmuMessageType>>>,
}

impl DanmuStream {
    pub async fn new(
        provider_type: ProviderType,
        identifier: &str,
        room_id: &str,
    ) -> Result<Self, DanmuStreamError> {
        let (tx, rx) = mpsc::unbounded_channel();
        let provider = new(provider_type, identifier, room_id).await?;
        Ok(Self {
            provider_type,
            identifier: identifier.to_string(),
            room_id: room_id.to_string(),
            provider: Arc::new(RwLock::new(provider)),
            tx,
            rx: Arc::new(RwLock::new(rx)),
        })
    }

    pub async fn start(&self) -> Result<(), DanmuStreamError> {
        self.provider.write().await.start(self.tx.clone()).await
    }

    pub async fn stop(&self) -> Result<(), DanmuStreamError> {
        self.provider.write().await.stop().await?;
        // close channel
        self.rx.write().await.close();
        Ok(())
    }

    pub async fn recv(&self) -> Result<Option<DanmuMessageType>, DanmuStreamError> {
        Ok(self.rx.write().await.recv().await)
    }
}
