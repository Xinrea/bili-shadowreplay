use std::sync::Arc;

use crate::{
    provider::{new, DanmakuProvider, ProviderType},
    DanmakuMessageType, DanmakuStreamError,
};
use tokio::sync::{mpsc, RwLock};

#[derive(Clone)]
pub struct DanmakuStream {
    pub provider_type: ProviderType,
    pub identifier: String,
    pub room_id: u64,
    pub provider: Arc<RwLock<Box<dyn DanmakuProvider>>>,
    tx: mpsc::UnboundedSender<DanmakuMessageType>,
    rx: Arc<RwLock<mpsc::UnboundedReceiver<DanmakuMessageType>>>,
}

impl DanmakuStream {
    pub async fn new(
        provider_type: ProviderType,
        identifier: &str,
        room_id: u64,
    ) -> Result<Self, DanmakuStreamError> {
        let (tx, rx) = mpsc::unbounded_channel();
        let provider = new(provider_type, identifier, room_id).await?;
        Ok(Self {
            provider_type,
            identifier: identifier.to_string(),
            room_id,
            provider: Arc::new(RwLock::new(provider)),
            tx,
            rx: Arc::new(RwLock::new(rx)),
        })
    }

    pub async fn start(&self) -> Result<(), DanmakuStreamError> {
        self.provider.write().await.start(self.tx.clone()).await
    }

    pub async fn stop(&self) -> Result<(), DanmakuStreamError> {
        self.provider.write().await.stop().await?;
        // close channel
        self.rx.write().await.close();
        Ok(())
    }

    pub async fn recv(&self) -> Result<Option<DanmakuMessageType>, DanmakuStreamError> {
        Ok(self.rx.write().await.recv().await)
    }
}
