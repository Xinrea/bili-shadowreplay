use crate::config::Config;
use axum::Router;
use std::{net::SocketAddr, sync::Arc};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

pub struct StaticServer {
    #[allow(dead_code)]
    pub handle: JoinHandle<()>,
    pub port: u16,
}

pub async fn start_static_server(
    config: Arc<RwLock<Config>>,
) -> Result<StaticServer, Box<dyn std::error::Error>> {
    let bind_addr = SocketAddr::from(([0, 0, 0, 0], 0));
    log::info!("Starting static server binding to {}", bind_addr);

    let listener = match tokio::net::TcpListener::bind(bind_addr).await {
        Ok(listener) => {
            match listener.local_addr() {
                Ok(addr) => log::info!("Static server listening on http://{}", addr),
                Err(e) => log::warn!("Unable to determine listening address: {}", e),
            }
            listener
        }
        Err(e) => {
            log::error!("Failed to bind static server: {}", e);
            log::error!("Please check if the port is already in use or try a different port");
            return Err(e.into());
        }
    };

    let port = listener.local_addr().unwrap().port();

    let output_path = config.read().await.output.clone();
    let cache_path = config.read().await.cache.clone();

    let handle = tokio::spawn(async move {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any);
        let router = Router::new()
            .layer(cors)
            .nest_service("/output", ServeDir::new(output_path))
            .nest_service("/cache", ServeDir::new(cache_path));

        if let Err(e) = axum::serve(listener, router).await {
            log::error!("Server error: {}", e);
        }
    });

    Ok(StaticServer { handle, port })
}
