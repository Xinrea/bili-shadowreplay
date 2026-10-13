//! Optional local MCP service, independent of the desktop/web API servers.

mod protocol;
mod tools;

use std::sync::{atomic::AtomicBool, Arc};

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use tokio::{
    net::TcpListener,
    sync::{oneshot, Mutex, RwLock},
    task::JoinHandle,
};

use crate::{config::Config, state::State};
use protocol::{Backend, RpcContext};

#[derive(Serialize)]
pub struct McpServerStatus {
    pub enabled: bool,
    pub running: bool,
    pub port: u16,
    pub url: String,
    pub error: Option<String>,
}

struct RunningServer {
    port: u16,
    active: Arc<AtomicBool>,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl RunningServer {
    fn start(listener: TcpListener, context: RpcContext) -> Self {
        let port = listener
            .local_addr()
            .expect("Bound MCP listener has an address")
            .port();
        let active = context.active.clone();
        let app = protocol::router(context, port);
        let (shutdown, stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
            {
                log::error!("MCP server error: {error}");
            }
        });
        Self {
            port,
            active,
            shutdown: Some(shutdown),
            task,
        }
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        self.active
            .store(false, std::sync::atomic::Ordering::Release);
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.abort();
    }
}

#[derive(Default)]
struct Runtime {
    server: Option<RunningServer>,
    error: Option<String>,
}

#[derive(Default)]
pub struct McpServer {
    runtime: Mutex<Runtime>,
}

impl McpServer {
    pub async fn restore(&self, state: State) {
        let enabled = state.config.read().await.mcp_enabled;
        if enabled {
            if let Err(error) = self.configure(state, true, None, false).await {
                log::error!("Failed to start MCP server: {error}");
            }
        }
    }

    pub async fn status(&self, state: &State) -> McpServerStatus {
        let runtime = self.runtime.lock().await;
        let config = state.config.read().await;
        Self::runtime_status(&runtime, config.mcp_enabled, config.mcp_port)
    }

    fn runtime_status(runtime: &Runtime, enabled: bool, configured_port: u16) -> McpServerStatus {
        let port = runtime
            .server
            .as_ref()
            .map_or(configured_port, |server| server.port);
        McpServerStatus {
            enabled,
            running: runtime
                .server
                .as_ref()
                .is_some_and(|server| !server.task.is_finished()),
            port,
            url: format!("http://127.0.0.1:{port}/mcp"),
            error: runtime.error.clone().or_else(|| {
                runtime
                    .server
                    .as_ref()
                    .filter(|server| server.task.is_finished())
                    .map(|_| "MCP Server 已停止，请重新启用".to_string())
            }),
        }
    }

    pub async fn set_enabled(
        &self,
        state: State,
        enabled: bool,
        port: Option<u16>,
    ) -> Result<McpServerStatus, String> {
        self.configure(state, enabled, port, true).await
    }

    async fn configure(
        &self,
        state: State,
        enabled: bool,
        port: Option<u16>,
        persist: bool,
    ) -> Result<McpServerStatus, String> {
        self.configure_with_backend(
            state.config.clone(),
            Arc::new(BsrBackend(state)),
            enabled,
            port,
            persist,
        )
        .await
    }

    async fn configure_with_backend(
        &self,
        config: Arc<RwLock<Config>>,
        backend: Arc<dyn Backend>,
        enabled: bool,
        port: Option<u16>,
        persist: bool,
    ) -> Result<McpServerStatus, String> {
        // Serialize configuration and listener changes, including concurrent UI requests.
        let mut runtime = self.runtime.lock().await;
        let port = port.unwrap_or(config.read().await.mcp_port);
        if port < 1024 {
            let error = "MCP 端口必须为 1024–65535 之间的整数".to_string();
            runtime.error = Some(error.clone());
            return Err(error);
        }
        if runtime
            .server
            .as_ref()
            .is_some_and(|server| server.task.is_finished())
        {
            runtime.server.take();
        }
        // Bind before saving or stopping the old service: a port conflict must
        // leave the previous address and configuration usable.
        let listener = if enabled
            && runtime
                .server
                .as_ref()
                .is_none_or(|server| server.port != port)
        {
            match TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await {
                Ok(listener) => Some(listener),
                Err(error) => {
                    let error = if error.kind() == std::io::ErrorKind::AddrInUse {
                        format!("MCP 端口 {port} 已被占用，请更换端口")
                    } else {
                        format!("无法启动 MCP Server：127.0.0.1:{port}，{error}")
                    };
                    runtime.error = Some(error.clone());
                    return Err(error);
                }
            }
        } else {
            None
        };

        if persist {
            let mut config = config.write().await;
            let mut next = config.clone();
            next.mcp_enabled = enabled;
            next.mcp_port = port;
            // A failed save leaves both the previous setting and listener intact.
            if let Err(error) = next.try_save() {
                runtime.error = Some(error.clone());
                return Err(error);
            }
            config.mcp_enabled = enabled;
            config.mcp_port = port;
        }

        if let Some(listener) = listener {
            Self::stop_runtime(&mut runtime).await;
            let active = Arc::new(AtomicBool::new(true));
            runtime.server = Some(RunningServer::start(
                listener,
                RpcContext {
                    backend,
                    active: active.clone(),
                },
            ));
            log::info!("MCP server listening on http://127.0.0.1:{port}/mcp");
        } else if !enabled {
            Self::stop_runtime(&mut runtime).await;
        }
        runtime.error = None;
        Ok(Self::runtime_status(&runtime, enabled, port))
    }

    async fn stop_runtime(runtime: &mut Runtime) {
        if let Some(mut server) = runtime.server.take() {
            server
                .active
                .store(false, std::sync::atomic::Ordering::Release);
            if let Some(shutdown) = server.shutdown.take() {
                let _ = shutdown.send(());
            }
            // Graceful shutdown closes idle keep-alive connections. Do not make
            // the settings switch wait indefinitely for an already running clip.
            if tokio::time::timeout(std::time::Duration::from_secs(1), &mut server.task)
                .await
                .is_err()
            {
                server.task.abort();
                let _ = (&mut server.task).await;
            }
            // Await cancellation so immediately enabling again can bind the same port.
            log::info!("MCP server stopped");
        }
    }

    pub async fn stop(&self) {
        Self::stop_runtime(&mut *self.runtime.lock().await).await;
    }
}

struct BsrBackend(State);

#[async_trait]
impl Backend for BsrBackend {
    fn definitions(&self) -> Vec<Value> {
        #[cfg(feature = "headless")]
        let readonly = self.0.readonly;
        #[cfg(not(feature = "headless"))]
        let readonly = false;
        tools::definitions(readonly)
    }

    async fn call(&self, name: &str, arguments: Value) -> Result<Value, String> {
        tools::call(&self.0, name, arguments).await
    }
}

#[cfg(test)]
mod mcp_server_runtime_tests {
    use super::*;
    use serde_json::json;
    use std::{
        net::SocketAddr,
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
        sync::Notify,
    };

    #[derive(Default)]
    struct TestBackend {
        block: bool,
        entered: Notify,
        release: Notify,
        completed: AtomicUsize,
    }

    #[async_trait]
    impl Backend for TestBackend {
        fn definitions(&self) -> Vec<Value> {
            vec![json!({"name": "wait", "inputSchema": {"type": "object", "properties": {}}})]
        }

        async fn call(&self, _name: &str, _arguments: Value) -> Result<Value, String> {
            self.entered.notify_one();
            if self.block {
                self.release.notified().await;
            }
            self.completed.fetch_add(1, Ordering::Relaxed);
            Ok(json!({"done": true}))
        }
    }

    async fn start(backend: Arc<TestBackend>) -> (Runtime, SocketAddr, Arc<AtomicBool>) {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let active = Arc::new(AtomicBool::new(true));
        let server = RunningServer::start(
            listener,
            RpcContext {
                backend,
                active: active.clone(),
            },
        );
        (
            Runtime {
                server: Some(server),
                error: None,
            },
            address,
            active,
        )
    }

    async fn ping(socket: &mut TcpStream, port: u16) {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let request = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
            body.len(),
        );
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            socket.read_exact(&mut byte).await.unwrap();
            headers.push(byte[0]);
        }
        let headers = String::from_utf8(headers).unwrap();
        assert!(headers.starts_with("HTTP/1.1 200"), "{headers}");
        let length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .and_then(|length| length.parse::<usize>().ok())
            })
            .unwrap();
        let mut response = vec![0; length];
        socket.read_exact(&mut response).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&response).unwrap()["result"],
            json!({})
        );
    }

    #[tokio::test]
    async fn stop_closes_keepalive_connections_and_allows_immediate_rebind() {
        let (mut runtime, address, active) = start(Arc::new(TestBackend::default())).await;
        let mut socket = TcpStream::connect(address).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), ping(&mut socket, address.port()))
            .await
            .unwrap();

        McpServer::stop_runtime(&mut runtime).await;
        assert!(runtime.server.is_none());
        assert!(!active.load(Ordering::Acquire));
        let mut byte = [0];
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );

        let listener = TcpListener::bind(address).await.unwrap();
        let active = Arc::new(AtomicBool::new(true));
        runtime.server = Some(RunningServer::start(
            listener,
            RpcContext {
                backend: Arc::new(TestBackend::default()),
                active,
            },
        ));
        let mut reconnected = TcpStream::connect(address).await.unwrap();
        tokio::time::timeout(
            Duration::from_secs(2),
            ping(&mut reconnected, address.port()),
        )
        .await
        .unwrap();
        McpServer::stop_runtime(&mut runtime).await;
    }

    #[tokio::test]
    async fn stop_is_bounded_and_allows_an_admitted_operation_to_finish() {
        let backend = Arc::new(TestBackend {
            block: true,
            ..TestBackend::default()
        });
        let (mut runtime, address, active) = start(backend.clone()).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let request = tokio::spawn(async move {
            client.post(format!("http://{address}/mcp"))
                .header("accept", "application/json, text/event-stream")
                .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "wait"}}))
                .send().await.unwrap().json::<Value>().await.unwrap()
        });
        tokio::time::timeout(Duration::from_secs(2), backend.entered.notified())
            .await
            .unwrap();

        tokio::time::timeout(
            Duration::from_secs(2),
            McpServer::stop_runtime(&mut runtime),
        )
        .await
        .unwrap();
        assert!(runtime.server.is_none());
        assert!(!active.load(Ordering::Acquire));
        assert_eq!(backend.completed.load(Ordering::Relaxed), 0);
        let rebound = TcpListener::bind(address).await.unwrap();
        backend.release.notify_one();
        let result = tokio::time::timeout(Duration::from_secs(2), request)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result["result"]["structuredContent"]["done"], true);
        assert_eq!(backend.completed.load(Ordering::Relaxed), 1);
        drop(rebound);
    }

    #[tokio::test]
    async fn dropping_server_closes_idle_connections_and_releases_listener() {
        let (mut runtime, address, active) = start(Arc::new(TestBackend::default())).await;
        let mut socket = TcpStream::connect(address).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), ping(&mut socket, address.port()))
            .await
            .unwrap();
        drop(runtime.server.take());
        assert!(!active.load(Ordering::Acquire));
        let mut byte = [0];
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        let listener = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match TcpListener::bind(address).await {
                    Ok(listener) => break listener,
                    Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                        tokio::task::yield_now().await
                    }
                    Err(error) => panic!("Failed to bind after dropping MCP server: {error}"),
                }
            }
        })
        .await
        .unwrap();
        drop(listener);
    }

    struct ConfigurationFixture {
        server: McpServer,
        config: Arc<RwLock<Config>>,
        backend: Arc<TestBackend>,
        directory: tempfile::TempDir,
    }

    impl ConfigurationFixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let config = Config::load(
                &directory.path().join("config.toml"),
                directory.path(),
                directory.path(),
            )
            .unwrap();
            Self {
                server: McpServer::default(),
                config: Arc::new(RwLock::new(config)),
                backend: Arc::new(TestBackend::default()),
                directory,
            }
        }

        async fn configure(
            &self,
            enabled: bool,
            port: Option<u16>,
        ) -> Result<McpServerStatus, String> {
            self.server
                .configure_with_backend(
                    self.config.clone(),
                    self.backend.clone(),
                    enabled,
                    port,
                    true,
                )
                .await
        }

        async fn status(&self) -> McpServerStatus {
            let runtime = self.server.runtime.lock().await;
            let config = self.config.read().await;
            McpServer::runtime_status(&runtime, config.mcp_enabled, config.mcp_port)
        }

        fn persisted(&self) -> Config {
            Config::load(
                &self.directory.path().join("config.toml"),
                self.directory.path(),
                self.directory.path(),
            )
            .unwrap()
        }
    }

    async fn available_port() -> u16 {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        listener.local_addr().unwrap().port()
    }

    async fn connect_and_ping(port: u16) -> TcpStream {
        let mut socket = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), ping(&mut socket, port))
            .await
            .unwrap();
        socket
    }

    #[tokio::test]
    async fn changing_port_persists_new_address_and_uses_actual_host_validation() {
        let fixture = ConfigurationFixture::new();
        let old_port = available_port().await;
        fixture.configure(true, Some(old_port)).await.unwrap();
        let mut old_client = connect_and_ping(old_port).await;
        let new_port = available_port().await;
        let changed = fixture.configure(true, Some(new_port)).await.unwrap();
        assert!(changed.enabled && changed.running);
        assert_eq!(changed.port, new_port);
        assert_eq!(changed.url, format!("http://127.0.0.1:{new_port}/mcp"));
        assert!(changed.error.is_none());
        assert_eq!(fixture.config.read().await.mcp_port, new_port);
        assert_eq!(fixture.persisted().mcp_port, new_port);

        let mut byte = [0];
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), old_client.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        let old_port_probe = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, old_port))
            .await
            .unwrap();
        let _new_client = connect_and_ping(new_port).await;
        let runtime = fixture.server.runtime.lock().await;
        let actual_status = McpServer::runtime_status(&runtime, true, old_port);
        assert_eq!(actual_status.port, new_port);
        assert_eq!(actual_status.url, changed.url);
        drop(runtime);

        let wrong_host = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(&changed.url)
            .header("host", format!("127.0.0.1:{old_port}"))
            .header("accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}))
            .send()
            .await
            .unwrap();
        assert_eq!(wrong_host.status(), axum::http::StatusCode::FORBIDDEN);
        fixture.server.stop().await;
        drop(old_port_probe);
    }

    #[tokio::test]
    async fn occupied_port_preserves_existing_listener_and_saved_settings() {
        let fixture = ConfigurationFixture::new();
        let old_port = available_port().await;
        fixture.configure(true, Some(old_port)).await.unwrap();
        let mut old_client = connect_and_ping(old_port).await;
        let occupied = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let occupied_port = occupied.local_addr().unwrap().port();
        assert!(fixture.configure(true, Some(occupied_port)).await.is_err());

        let status = fixture.status().await;
        assert!(status.enabled && status.running);
        assert_eq!(status.port, old_port);
        assert_eq!(status.url, format!("http://127.0.0.1:{old_port}/mcp"));
        assert!(status.error.is_some());
        assert_eq!(fixture.config.read().await.mcp_port, old_port);
        let saved = fixture.persisted();
        assert!(saved.mcp_enabled);
        assert_eq!(saved.mcp_port, old_port);
        tokio::time::timeout(Duration::from_secs(2), ping(&mut old_client, old_port))
            .await
            .unwrap();
        fixture.server.stop().await;
        drop(occupied);
    }

    #[tokio::test]
    async fn save_failure_and_invalid_port_leave_existing_service_unchanged() {
        let fixture = ConfigurationFixture::new();
        let old_port = available_port().await;
        fixture.configure(true, Some(old_port)).await.unwrap();
        let mut old_client = connect_and_ping(old_port).await;
        for invalid in [0, 1023] {
            assert!(fixture.configure(true, Some(invalid)).await.is_err());
            assert_eq!(fixture.status().await.port, old_port);
            assert_eq!(fixture.config.read().await.mcp_port, old_port);
        }

        let new_port = available_port().await;
        fixture.config.write().await.config_path =
            fixture.directory.path().to_string_lossy().into_owned();
        assert!(fixture.configure(true, Some(new_port)).await.is_err());
        let status = fixture.status().await;
        assert!(status.enabled && status.running);
        assert_eq!(status.port, old_port);
        assert_eq!(fixture.config.read().await.mcp_port, old_port);
        assert_eq!(fixture.persisted().mcp_port, old_port);
        let released_candidate = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, new_port))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), ping(&mut old_client, old_port))
            .await
            .unwrap();
        assert!(fixture.configure(false, None).await.is_err());
        assert!(fixture.status().await.running);
        tokio::time::timeout(Duration::from_secs(2), ping(&mut old_client, old_port))
            .await
            .unwrap();
        fixture.server.stop().await;
        drop(released_candidate);
    }

    #[tokio::test]
    async fn disabled_port_changes_save_without_binding_and_restart_reuses_port() {
        let fixture = ConfigurationFixture::new();
        let reservation = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = reservation.local_addr().unwrap().port();
        let disabled = fixture.configure(false, Some(port)).await.unwrap();
        assert!(!disabled.enabled && !disabled.running);
        assert_eq!(disabled.port, port);
        assert_eq!(disabled.url, format!("http://127.0.0.1:{port}/mcp"));
        let saved = fixture.persisted();
        assert!(!saved.mcp_enabled);
        assert_eq!(saved.mcp_port, port);
        drop(reservation);
        let enabled = fixture.configure(true, None).await.unwrap();
        assert_eq!(enabled.port, port);
        assert!(fixture.persisted().mcp_enabled);
        fixture.server.stop().await;

        let restarted = McpServer::default();
        let saved_config = Arc::new(RwLock::new(fixture.persisted()));
        let restored = restarted
            .configure_with_backend(saved_config, fixture.backend.clone(), true, None, false)
            .await
            .unwrap();
        assert!(restored.enabled && restored.running);
        assert_eq!(restored.url, enabled.url);
        let _client = connect_and_ping(port).await;
        restarted.stop().await;
    }
}
