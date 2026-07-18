use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub ha: HaConfig,
    pub sprinkler: SprinklerConfig,
    pub zones: Vec<ZoneConfig>,
    #[serde(default)]
    pub meters: Vec<MeterConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HaConfig {
    pub url: String,
    pub token: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SprinklerConfig {
    #[serde(default = "default_max_duration")]
    pub max_duration_secs: u64,
    #[serde(default = "default_true")]
    pub mutex: bool,
    #[serde(default = "default_true")]
    pub invert_logic: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ZoneConfig {
    pub id: String,
    pub name: String,
    pub gpio: u8,
    pub max_duration_secs: Option<u64>,
    /// Icon hint for the HA valve entity ("sprinkler", "drip", "hose", …).
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MeterConfig {
    pub id: String,
    pub name: String,
    pub gpio: u8,
    /// Unit of the meter (e.g. "L"). Defaults to "L".
    pub unit: Option<String>,
    /// HA device_class for the sensor (e.g. "water").
    pub device_class: Option<String>,
    /// Debounce window for the pulse input. Defaults to 200ms.
    pub debounce_ms: Option<u64>,
    /// Value added to the HA sensor at each pulse. Defaults to 1.
    pub increment_per_pulse: Option<u64>,
}

fn default_bind() -> String {
    "0.0.0.0".into()
}
fn default_port() -> u16 {
    8090
}
fn default_max_duration() -> u64 {
    1800
}
fn default_true() -> bool {
    true
}

impl Config {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;

        anyhow::ensure!(
            !config.zones.is_empty(),
            "At least one zone must be configured"
        );

        // Check for duplicate IDs across zones and meters, and GPIO collisions.
        let mut seen_ids = std::collections::HashSet::new();
        let mut seen_gpios = std::collections::HashSet::new();

        for z in &config.zones {
            anyhow::ensure!(seen_ids.insert(z.id.clone()), "Duplicate id: {}", z.id);
            anyhow::ensure!(
                seen_gpios.insert(z.gpio),
                "Duplicate GPIO {} (zone '{}')",
                z.gpio,
                z.id
            );
        }
        for m in &config.meters {
            anyhow::ensure!(seen_ids.insert(m.id.clone()), "Duplicate id: {}", m.id);
            anyhow::ensure!(
                seen_gpios.insert(m.gpio),
                "Duplicate GPIO {} (meter '{}')",
                m.gpio,
                m.id
            );
        }

        Ok(config)
    }
}

impl ZoneConfig {
    pub fn max_duration(&self, default_secs: u64) -> Duration {
        Duration::from_secs(self.max_duration_secs.unwrap_or(default_secs))
    }
}
