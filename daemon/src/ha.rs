use reqwest::Client;
use tracing::{debug, warn};

use crate::sprinkler::ZoneStatus;

#[derive(Clone)]
pub struct HaClient {
    client: Client,
    base_url: String,
    token: String,
}

impl HaClient {
    pub fn new(url: &str, token: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: url.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }

    /// POST an event of the given type on HA's event bus. Returns success.
    async fn fire_event<T: serde::Serialize + ?Sized>(&self, event_type: &str, payload: &T) -> bool {
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

    /// Fire a `waterpi_switch_update` event on HA's event bus when a physical
    /// switch changes state.
    pub async fn push_switch_state(&self, id: &str, is_on: bool) {
        let payload = serde_json::json!({ "id": id, "is_on": is_on });
        if self.fire_event("waterpi_switch_update", &payload).await {
            debug!(switch = id, is_on, "Pushed switch state to HA");
        }
    }
}
