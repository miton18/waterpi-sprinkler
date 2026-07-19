use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub ha: HaConfig,
    pub sprinkler: SprinklerConfig,
    pub zones: Vec<ZoneConfig>,
    #[serde(default)]
    pub switches: Vec<SwitchConfig>,
    #[serde(default)]
    pub display: Option<DisplayConfig>,
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

#[derive(Debug, Deserialize, Clone)]
pub struct SwitchConfig {
    pub id: String,
    pub name: String,
    pub gpio: u8,
    /// Debounce window in ms. Defaults to 50. 0 disables kernel debounce.
    pub debounce_ms: Option<u64>,
    /// Invert the on/off mapping (default: closed to GND = ON).
    pub inverted: Option<bool>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DisplayConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// I2C bus number (/dev/i2c-N). Defaults to 1.
    #[serde(default = "default_i2c_bus")]
    pub i2c_bus: u8,
    /// I2C address of the SSD1306. Defaults to 0x3C.
    #[serde(default = "default_i2c_address")]
    pub address: u8,
    /// HA weather entity shown on the idle view (e.g. "weather.maison").
    /// None: a clock is shown instead.
    pub weather_entity: Option<String>,
    /// Render interval in seconds. Defaults to 1.
    #[serde(default = "default_refresh_secs")]
    pub refresh_secs: u64,
    /// Weather poll interval in seconds. Defaults to 300.
    #[serde(default = "default_weather_refresh_secs")]
    pub weather_refresh_secs: u64,
}

fn default_bind() -> String {
    "0.0.0.0".into()
}
fn default_i2c_bus() -> u8 {
    1
}
fn default_i2c_address() -> u8 {
    0x3C
}
fn default_refresh_secs() -> u64 {
    1
}
fn default_weather_refresh_secs() -> u64 {
    300
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

        // Check for duplicate IDs and GPIO collisions across zones and switches
        let mut seen_ids = std::collections::HashSet::new();
        let mut seen_gpios = std::collections::HashSet::new();
        for z in &config.zones {
            anyhow::ensure!(seen_ids.insert(&z.id), "Duplicate id: {} (zone)", z.id);
            anyhow::ensure!(
                seen_gpios.insert(z.gpio),
                "Duplicate GPIO {} (zone '{}')",
                z.gpio,
                z.id
            );
        }
        for s in &config.switches {
            anyhow::ensure!(seen_ids.insert(&s.id), "Duplicate id: {} (switch)", s.id);
            anyhow::ensure!(
                seen_gpios.insert(s.gpio),
                "Duplicate GPIO {} (switch '{}')",
                s.gpio,
                s.id
            );
        }

        if let Some(d) = &config.display {
            anyhow::ensure!(d.refresh_secs >= 1, "display.refresh_secs must be >= 1");
            anyhow::ensure!(
                d.weather_refresh_secs >= 10,
                "display.weather_refresh_secs must be >= 10"
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
