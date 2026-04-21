mod api;
mod config;
mod ha;
mod sprinkler;

use std::net::SocketAddr;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("waterpi_sprinkler=info")),
        )
        .init();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config.toml".into());

    let config = config::Config::load(&config_path)?;
    info!(
        zones = config.zones.len(),
        port = config.server.port,
        "Loaded configuration"
    );

    let ha_client = ha::HaClient::new(&config.ha.url, &config.ha.token);
    let ctrl = sprinkler::create(&config, ha_client)?;

    let app = api::router(ctrl.clone());
    let addr: SocketAddr = format!("{}:{}", config.server.bind, config.server.port).parse()?;

    info!(%addr, "Starting waterpi-sprinkler");
    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Graceful shutdown: close all valves on SIGTERM / Ctrl-C
    let ctrl_shutdown = ctrl.clone();

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            info!("Shutdown signal received — closing all valves");
            sprinkler::close_all(&ctrl_shutdown).await;
            info!("All valves closed, exiting");
        })
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => {}
            _ = sigterm.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
    }
}
