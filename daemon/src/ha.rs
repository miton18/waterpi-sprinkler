use reqwest::Client;
use serde::Serialize;
use tracing::{debug, warn};

use crate::sprinkler::ZoneStatus;

#[derive(Clone)]
pub struct HaClient {
    client: Client,
    base_url: String,
    token: String,
}

#[derive(Serialize)]
struct MeterPulseEvent<'a> {
    id: &'a str,
    increment: u64,
    unit: &'a str,
}

impl HaClient {
    pub fn new(url: &str, token: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: url.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }

    async fn fire_event<T: Serialize>(&self, event_type: &str, payload: &T) -> bool {
        let url = format!("{}/api/events/{}", self.base_url, event_type);
        match self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .json(payload)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => true,
            Ok(resp) => {
                warn!(event = event_type, status = %resp.status(), "HA rejected event");
                false
            }
            Err(e) => {
                warn!(event = event_type, error = %e, "Failed to fire HA event (is HA running?)");
                false
            }
        }
    }

    /// Fire a `waterpi_sprinkler_update` event on HA's event bus.
    /// The custom component listens on this event type to refresh entity state
    /// immediately (instead of waiting for the next poll).
    pub async fn push_state(&self, zone: &ZoneStatus) {
        if self.fire_event("waterpi_sprinkler_update", zone).await {
            debug!(zone = %zone.id, is_open = zone.is_open, "Pushed state to HA");
        }
    }

    /// Fire a `waterpi_meter_pulse` event on HA's event bus. The HA sensor
    /// accumulates the increments — the daemon keeps no state.
    pub async fn push_meter_pulse(&self, id: &str, increment: u64, unit: &str) {
        let payload = MeterPulseEvent {
            id,
            increment,
            unit,
        };
        if self.fire_event("waterpi_meter_pulse", &payload).await {
            debug!(meter = id, increment, "Pushed meter pulse to HA");
        }
    }
}
