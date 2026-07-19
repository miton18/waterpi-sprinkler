//! Current-weather cache for the OLED display, fed from Home Assistant.
//!
//! A background task polls `GET /api/states/{weather_entity}` and keeps the
//! last successful reading in a shared cache. Failures never clear the cache:
//! the display keeps showing the last known value ("--" if never fetched).

use std::sync::{Arc, RwLock};
use std::time::Duration;

use tokio::time::MissedTickBehavior;
use tracing::{debug, warn};

use crate::ha::HaClient;

#[derive(Debug, Clone)]
pub struct Weather {
    /// Raw HA condition state, e.g. "partlycloudy".
    pub condition: String,
    pub temperature: Option<f64>,
}

/// Written by the poll task (~every 5 min), read by the render loop (~1 Hz).
/// Never held across an await point, so a std RwLock is fine.
pub type WeatherCache = Arc<RwLock<Option<Weather>>>;

/// Map an HA weather condition to a French label (fits 21 columns in 6x10).
pub fn condition_fr(condition: &str) -> &str {
    match condition {
        "sunny" => "Ensoleillé",
        "clear-night" => "Nuit claire",
        "partlycloudy" => "Partiellement nuageux",
        "cloudy" => "Nuageux",
        "rainy" => "Pluie",
        "pouring" => "Pluie forte",
        "lightning" => "Orage",
        "lightning-rainy" => "Orage et pluie",
        "snowy" => "Neige",
        "snowy-rainy" => "Neige et pluie",
        "hail" => "Grêle",
        "fog" => "Brouillard",
        "windy" | "windy-variant" => "Venteux",
        "exceptional" => "Exceptionnel",
        other => other,
    }
}

pub fn spawn_poller(
    ha: HaClient,
    entity_id: String,
    cache: WeatherCache,
    every: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(every);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match ha.get_state(&entity_id).await {
                Ok(entity) => {
                    let weather = Weather {
                        condition: entity.state,
                        temperature: entity.attributes.get("temperature").and_then(|v| v.as_f64()),
                    };
                    debug!(entity = %entity_id, condition = %weather.condition, "Weather updated");
                    *cache.write().unwrap() = Some(weather);
                }
                Err(e) => {
                    warn!(entity = %entity_id, error = %e, "Weather fetch failed — keeping last value");
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_conditions_are_translated() {
        assert_eq!(condition_fr("sunny"), "Ensoleillé");
        assert_eq!(condition_fr("partlycloudy"), "Partiellement nuageux");
        assert_eq!(condition_fr("lightning-rainy"), "Orage et pluie");
        assert_eq!(condition_fr("windy-variant"), "Venteux");
    }

    #[test]
    fn unknown_condition_passes_through() {
        assert_eq!(condition_fr("weird-state"), "weird-state");
    }

    #[test]
    fn labels_fit_the_display_width() {
        for c in [
            "sunny",
            "clear-night",
            "partlycloudy",
            "cloudy",
            "rainy",
            "pouring",
            "lightning",
            "lightning-rainy",
            "snowy",
            "snowy-rainy",
            "hail",
            "fog",
            "windy",
            "exceptional",
        ] {
            assert!(condition_fr(c).chars().count() <= 21, "label too long for {c}");
        }
    }
}
