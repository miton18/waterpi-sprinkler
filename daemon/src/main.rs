mod api;
mod config;
mod display;
mod ha;
mod meter;
mod sprinkler;
mod switch;
mod weather;

use std::net::SocketAddr;
use tracing::{info, warn};
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
        meters = config.meters.len(),
        switches = config.switches.len(),
        port = config.server.port,
        "Loaded configuration"
    );

    let ha_client = ha::HaClient::new(&config.ha.url, &config.ha.token);
    let ctrl = sprinkler::create(&config, ha_client.clone())?;
    let meters = meter::create(&config, ha_client.clone())?;
    let switches = switch::create(&config, ha_client.clone())?;

    // Optional OLED display: best-effort, the daemon runs fine without it.
    let mut display_handle: Option<display::DisplayHandle> = None;
    if let Some(dc) = config.display.as_ref().filter(|d| d.enabled) {
        let weather_cache: weather::WeatherCache = Default::default();
        if let Some(entity) = &dc.weather_entity {
            weather::spawn_poller(
                ha_client.clone(),
                entity.clone(),
                weather_cache.clone(),
                std::time::Duration::from_secs(dc.weather_refresh_secs),
            );
        }
        match display::spawn(dc, ctrl.clone(), switches.clone(), weather_cache) {
            Ok(h) => display_handle = Some(h),
            Err(e) => warn!(error = %e, "OLED init failed — continuing without display"),
        }
    }

    let app = api::router(ctrl.clone(), meters, switches);
    let addr: SocketAddr = format!("{}:{}", config.server.bind, config.server.port).parse()?;

    info!(%addr, "Starting waterpi-sprinkler");
    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Graceful shutdown: close all valves
    let ctrl_shutdown = ctrl.clone();

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            info!("Shutdown signal received — closing all valves");
            sprinkler::close_all(&ctrl_shutdown).await;
            if let Some(h) = display_handle {
                h.shutdown().await;
            }
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
