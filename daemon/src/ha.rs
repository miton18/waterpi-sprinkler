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

    /// Fire a `waterpi_sprinkler_update` event on HA's event bus.
    /// The custom component listens on this event type to refresh entity state
    /// immediately (instead of waiting for the next poll).
    pub async fn push_state(&self, zone: &ZoneStatus) {
        let url = format!("{}/api/events/waterpi_sprinkler_update", self.base_url);

        match self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .json(zone)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                debug!(zone = %zone.id, is_open = zone.is_open, "Pushed state to HA");
            }
            Ok(resp) => {
                warn!(zone = %zone.id, status = %resp.status(), "HA rejected state push");
            }
            Err(e) => {
                warn!(zone = %zone.id, error = %e, "Failed to push state to HA (is HA running?)");
            }
        }
    }
}
