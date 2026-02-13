mod config;
mod engine;
mod handlers;
mod health;
mod metrics;
mod policies;
mod tls;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;
use clap::Parser;
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder as HttpBuilder;
use hyper_util::service::TowerToHyperService;
use prometheus_client::registry::Registry;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_rustls::TlsAcceptor;
use tracing::{error, info};

#[derive(Parser)]
#[command(name = "k8s-sentinel", about = "Kubernetes admission webhook")]
struct Cli {
    /// Path to the configuration file
    #[arg(long, default_value = "/etc/sentinel/policies.yaml", env = "SENTINEL_CONFIG")]
    config: String,
}

async fn shutdown_signal(shutdown_tx: watch::Sender<()>) {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => info!("received CTRL+C, starting graceful shutdown"),
            _ = sigterm.recv() => info!("received SIGTERM, starting graceful shutdown"),
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.expect("failed to listen for ctrl_c");
        info!("received CTRL+C, starting graceful shutdown");
    }

    let _ = shutdown_tx.send(());
}

async fn run_https_server(
    addr: SocketAddr,
    tls_acceptor: TlsAcceptor,
    router: Router,
    ready: Arc<AtomicBool>,
    mut shutdown_rx: watch::Receiver<()>,
) {
    let listener = TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind HTTPS on {addr}: {e}"));

    info!(%addr, "HTTPS webhook server listening");
    ready.store(true, Ordering::Relaxed);

    loop {
        let (tcp_stream, remote_addr) = tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok(conn) => conn,
                    Err(e) => {
                        error!("failed to accept TCP connection: {e}");
                        continue;
                    }
                }
            }
            _ = shutdown_rx.changed() => {
                info!("HTTPS server shutting down");
                break;
            }
        };

        let tls_acceptor = tls_acceptor.clone();
        let router = router.clone();

        tokio::spawn(async move {
            let tls_stream = match tls_acceptor.accept(tcp_stream).await {
                Ok(stream) => stream,
                Err(e) => {
                    error!(%remote_addr, "TLS handshake failed: {e}");
                    return;
                }
            };

            let io = TokioIo::new(tls_stream);
            let service = TowerToHyperService::new(router.into_service());

            if let Err(e) = HttpBuilder::new(hyper_util::rt::TokioExecutor::new())
                .serve_connection(io, service)
                .await
            {
                error!(%remote_addr, "error serving connection: {e}");
            }
        });
    }
}

async fn run_http_server(
    addr: SocketAddr,
    router: Router,
    mut shutdown_rx: watch::Receiver<()>,
) {
    let listener = TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind HTTP on {addr}: {e}"));

    info!(%addr, "HTTP metrics/health server listening");

    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.changed().await;
            info!("HTTP server shutting down");
        })
        .await
        .unwrap_or_else(|e| error!("HTTP server error: {e}"));
}

#[tokio::main]
async fn main() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install default CryptoProvider");

    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = config::SentinelConfig::load(&cli.config).unwrap_or_else(|e| {
        eprintln!("Failed to load config from {}: {e}", cli.config);
        std::process::exit(1);
    });

    info!(
        listen_addr = %config.listen_addr,
        metrics_addr = %config.metrics_addr,
        log_level = %config.log_level,
        policies.resource_limits.enabled = config.policies.resource_limits.enabled,
        policies.resource_limits.mode = ?config.policies.resource_limits.mode,
        policies.image_registry.enabled = config.policies.image_registry.enabled,
        policies.image_registry.mode = ?config.policies.image_registry.mode,
        policies.labels.enabled = config.policies.labels.enabled,
        policies.labels.mode = ?config.policies.labels.mode,
        policies.topology_spread.enabled = config.policies.topology_spread.enabled,
        policies.topology_spread.mode = ?config.policies.topology_spread.mode,
        "k8s-sentinel starting"
    );

    let tls_config = tls::load_tls_config(&config.tls_cert_path, &config.tls_key_path)
        .unwrap_or_else(|e| {
            eprintln!("Failed to load TLS config: {e}");
            std::process::exit(1);
        });
    let tls_acceptor = TlsAcceptor::from(tls_config);

    let mut registry = Registry::default();
    let sentinel_metrics = metrics::SentinelMetrics::new(&mut registry, &config.policies);
    let registry = Arc::new(registry);

    let engine = engine::PolicyEngine::new(config.policies.clone());

    let app_state = Arc::new(handlers::AppState {
        engine,
        metrics: sentinel_metrics,
    });

    let webhook_router = Router::new()
        .route("/validate", post(handlers::handle_validate))
        .route("/mutate", post(handlers::handle_mutate))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(app_state);

    let ready = Arc::new(AtomicBool::new(false));
    let health_state = Arc::new(health::HealthState {
        registry,
        ready: ready.clone(),
    });

    let metrics_router = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
        .route("/metrics", get(health::metrics_handler))
        .with_state(health_state);

    let listen_addr: SocketAddr = config.listen_addr.parse().unwrap_or_else(|e| {
        eprintln!("Invalid listen_addr '{}': {e}", config.listen_addr);
        std::process::exit(1);
    });
    let metrics_addr: SocketAddr = config.metrics_addr.parse().unwrap_or_else(|e| {
        eprintln!("Invalid metrics_addr '{}': {e}", config.metrics_addr);
        std::process::exit(1);
    });

    let (shutdown_tx, shutdown_rx) = watch::channel(());

    let https_shutdown_rx = shutdown_rx.clone();
    let http_shutdown_rx = shutdown_rx;

    tokio::spawn(shutdown_signal(shutdown_tx));

    tokio::join!(
        run_https_server(listen_addr, tls_acceptor, webhook_router, ready, https_shutdown_rx),
        run_http_server(metrics_addr, metrics_router, http_shutdown_rx),
    );

    info!("k8s-sentinel shut down gracefully");
}
