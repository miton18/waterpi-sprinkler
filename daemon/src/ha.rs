use reqwest::Client;
use tracing::{debug, warn};

use crate::sprinkler::ZoneStatus;

#[derive(Clone)]
pub struct HaClient {
    client: Client,
    base_url: String,
    token: String,
}

/// Subset of HA's `GET /api/states/{entity_id}` response.
#[derive(Debug, serde::Deserialize)]
pub struct EntityState {
    pub state: String,
    #[serde(default)]
    pub attributes: serde_json::Value,
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

    /// Fetch the current state of an HA entity (`GET /api/states/{entity_id}`).
    /// Errors bubble up; the caller decides how tolerant to be.
    pub async fn get_state(&self, entity_id: &str) -> anyhow::Result<EntityState> {
        let url = format!("{}/api/states/{}", self.base_url, entity_id);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
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
