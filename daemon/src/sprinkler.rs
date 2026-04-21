use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rppal::gpio::{Gpio, OutputPin};
use serde::Serialize;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::Config;
use crate::ha::HaClient;

fn state_dir() -> PathBuf {
    match std::env::var("STATE_DIRECTORY") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => PathBuf::from("/var/lib/sprinkler"),
    }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

pub type Sprinkler = Arc<Mutex<SprinklerInner>>;

#[derive(Debug, Clone, Serialize)]
pub struct ZoneStatus {
    pub id: String,
    pub name: String,
    pub gpio: u8,
    pub is_open: bool,
    pub opened_at: Option<String>,
    pub open_duration_secs: Option<u64>,
    pub max_duration_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Last time this zone was opened (persists after close).
    pub last_opened_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

struct Zone {
    id: String,
    name: String,
    gpio_num: u8,
    kind: Option<String>,
    max_duration: Duration,
    is_open: bool,
    opened_at: Option<Instant>,
    last_opened_at: Option<chrono::DateTime<chrono::Utc>>,
    pin: OutputPin,
    timeout_handle: Option<JoinHandle<()>>,
}

pub struct SprinklerInner {
    zones: Vec<Zone>,
    mutex_enabled: bool,
    invert_logic: bool,
    ha_client: HaClient,
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

pub fn create(config: &Config, ha_client: HaClient) -> anyhow::Result<Sprinkler> {
    let gpio = Gpio::new()?;
    let invert = config.sprinkler.invert_logic;
    let mut zones = Vec::new();

    // Ensure state directory exists
    let sd = state_dir();
    if let Err(e) = std::fs::create_dir_all(&sd) {
        warn!("Cannot create state dir {}: {}", sd.display(), e);
    }

    for zc in &config.zones {
        let mut pin = gpio.get(zc.gpio)?.into_output();

        // Start with all valves closed
        if invert {
            pin.set_high();
        } else {
            pin.set_low();
        }

        // Restore last_opened_at from disk
        let last_opened_at = load_last_opened(&zc.id);

        zones.push(Zone {
            id: zc.id.clone(),
            name: zc.name.clone(),
            gpio_num: zc.gpio,
            kind: zc.kind.clone(),
            max_duration: zc.max_duration(config.sprinkler.max_duration_secs),
            is_open: false,
            opened_at: None,
            last_opened_at,
            pin,
            timeout_handle: None,
        });

        info!(zone = %zc.id, gpio = zc.gpio, last_opened = ?last_opened_at, "Zone initialized (closed)");
    }

    Ok(Arc::new(Mutex::new(SprinklerInner {
        zones,
        mutex_enabled: config.sprinkler.mutex,
        invert_logic: invert,
        ha_client,
    })))
}

// ---------------------------------------------------------------------------
// Public operations
// ---------------------------------------------------------------------------

pub async fn open_zone(sprinkler: &Sprinkler, zone_id: &str) -> Result<ZoneStatus, String> {
    let mut inner = sprinkler.lock().await;
    let idx = inner
        .zone_index(zone_id)
        .ok_or_else(|| format!("Zone '{}' not found", zone_id))?;

    // Already open → noop
    if inner.zones[idx].is_open {
        return Ok(inner.zone_status(idx));
    }

    // Mutex: close any other open zone first
    if inner.mutex_enabled {
        let open_indices: Vec<usize> = inner
            .zones
            .iter()
            .enumerate()
            .filter(|(i, z)| z.is_open && *i != idx)
            .map(|(i, _)| i)
            .collect();

        for i in open_indices {
            inner.close_zone_inner(i);
            let status = inner.zone_status(i);
            inner.ha_client.push_state(&status).await;
        }
    }

    // Open the requested zone
    let now = chrono::Utc::now();
    inner.set_pin_open(idx);
    inner.zones[idx].is_open = true;
    inner.zones[idx].opened_at = Some(Instant::now());
    inner.zones[idx].last_opened_at = Some(now);
    save_last_opened(zone_id, now);
    info!(zone = %zone_id, "Zone opened");

    // Spawn a safety timeout
    let max_dur = inner.zones[idx].max_duration;
    let sprinkler_clone = Arc::clone(sprinkler);
    let zone_id_owned = zone_id.to_string();

    let handle = tokio::spawn(async move {
        tokio::time::sleep(max_dur).await;
        warn!(zone = %zone_id_owned, "Max duration reached — auto-closing");
        // We ignore errors here; the zone may already be closed.
        let _ = close_zone(&sprinkler_clone, &zone_id_owned).await;
    });
    inner.zones[idx].timeout_handle = Some(handle);

    let status = inner.zone_status(idx);
    inner.ha_client.push_state(&status).await;
    Ok(status)
}

pub async fn close_zone(sprinkler: &Sprinkler, zone_id: &str) -> Result<ZoneStatus, String> {
    let mut inner = sprinkler.lock().await;
    let idx = inner
        .zone_index(zone_id)
        .ok_or_else(|| format!("Zone '{}' not found", zone_id))?;

    inner.close_zone_inner(idx);

    let status = inner.zone_status(idx);
    inner.ha_client.push_state(&status).await;
    Ok(status)
}

pub async fn close_all(sprinkler: &Sprinkler) -> Vec<ZoneStatus> {
    let mut inner = sprinkler.lock().await;
    let count = inner.zones.len();

    for i in 0..count {
        inner.close_zone_inner(i);
    }

    let statuses = inner.all_statuses();
    for s in &statuses {
        inner.ha_client.push_state(s).await;
    }
    statuses
}

pub async fn get_all(sprinkler: &Sprinkler) -> Vec<ZoneStatus> {
    sprinkler.lock().await.all_statuses()
}

pub async fn get_zone(sprinkler: &Sprinkler, zone_id: &str) -> Result<ZoneStatus, String> {
    let inner = sprinkler.lock().await;
    let idx = inner
        .zone_index(zone_id)
        .ok_or_else(|| format!("Zone '{}' not found", zone_id))?;
    Ok(inner.zone_status(idx))
}

// ---------------------------------------------------------------------------
// SprinklerInner helpers
// ---------------------------------------------------------------------------

impl SprinklerInner {
    fn zone_index(&self, id: &str) -> Option<usize> {
        self.zones.iter().position(|z| z.id == id)
    }

    fn set_pin_open(&mut self, idx: usize) {
        if self.invert_logic {
            self.zones[idx].pin.set_low();
        } else {
            self.zones[idx].pin.set_high();
        }
    }

    fn set_pin_closed(&mut self, idx: usize) {
        if self.invert_logic {
            self.zones[idx].pin.set_high();
        } else {
            self.zones[idx].pin.set_low();
        }
    }

    fn close_zone_inner(&mut self, idx: usize) {
        if !self.zones[idx].is_open {
            return;
        }

        self.set_pin_closed(idx);
        self.zones[idx].is_open = false;
        self.zones[idx].opened_at = None;

        if let Some(handle) = self.zones[idx].timeout_handle.take() {
            handle.abort();
        }

        info!(zone = %self.zones[idx].id, "Zone closed");
    }

    fn zone_status(&self, idx: usize) -> ZoneStatus {
        let z = &self.zones[idx];
        ZoneStatus {
            id: z.id.clone(),
            name: z.name.clone(),
            gpio: z.gpio_num,
            is_open: z.is_open,
            opened_at: z.opened_at.map(|t| {
                let elapsed = t.elapsed();
                let wall =
                    chrono::Utc::now() - chrono::Duration::from_std(elapsed).unwrap_or_default();
                wall.to_rfc3339()
            }),
            open_duration_secs: z.opened_at.map(|t| t.elapsed().as_secs()),
            max_duration_secs: z.max_duration.as_secs(),
            kind: z.kind.clone(),
            last_opened_at: z.last_opened_at.map(|t| t.to_rfc3339()),
        }
    }

    fn all_statuses(&self) -> Vec<ZoneStatus> {
        (0..self.zones.len()).map(|i| self.zone_status(i)).collect()
    }
}

// ---------------------------------------------------------------------------
// Disk persistence for last_opened_at
// ---------------------------------------------------------------------------

fn state_path(zone_id: &str) -> PathBuf {
    state_dir().join(format!("{}.state", zone_id))
}

fn load_last_opened(zone_id: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let path = state_path(zone_id);
    let content = std::fs::read_to_string(&path).ok()?;
    content.trim().parse().ok()
}

fn save_last_opened(zone_id: &str, dt: chrono::DateTime<chrono::Utc>) {
    let path = state_path(zone_id);
    if let Err(e) = std::fs::write(&path, dt.to_rfc3339()) {
        warn!(zone = %zone_id, "Failed to persist last_opened_at to {}: {}", path.display(), e);
    }
}
