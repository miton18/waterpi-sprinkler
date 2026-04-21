use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub ha: HaConfig,
    pub sprinkler: SprinklerConfig,
    pub zones: Vec<ZoneConfig>,
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
    /// Icon hint: "sprinkler", "water", "drip", etc.
    pub kind: Option<String>,
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

        // Check for duplicate zone IDs
        let mut seen = std::collections::HashSet::new();
        for z in &config.zones {
            anyhow::ensure!(seen.insert(&z.id), "Duplicate zone id: {}", z.id);
        }

        Ok(config)
    }
}

impl ZoneConfig {
    pub fn max_duration(&self, default_secs: u64) -> Duration {
        Duration::from_secs(self.max_duration_secs.unwrap_or(default_secs))
    }
}
