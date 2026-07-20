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
    /// I2C address of the panel. Defaults to 0x3C.
    #[serde(default = "default_i2c_address")]
    pub address: u8,
    /// OLED driver: "ssd1306" (0.96", default) or "ssd1309" (2.42").
    pub driver: Option<String>,
    /// BCM GPIO wired to the panel's RST/RES pin (ssd1309 only; omit for
    /// 4-pin I2C modules with on-board auto-reset).
    pub reset_gpio: Option<u8>,
    /// Contrast 0-255 applied at init (default: the driver's init default).
    pub contrast: Option<u8>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayDriver {
    Ssd1306,
    Ssd1309,
}

impl DisplayDriver {
    pub fn name(self) -> &'static str {
        match self {
            DisplayDriver::Ssd1306 => "ssd1306",
            DisplayDriver::Ssd1309 => "ssd1309",
        }
    }
}

impl DisplayConfig {
    /// Parsed driver; validation guarantees the string is valid.
    pub fn driver(&self) -> DisplayDriver {
        match self.driver.as_deref() {
            Some("ssd1309") => DisplayDriver::Ssd1309,
            _ => DisplayDriver::Ssd1306,
        }
    }
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
        Self::parse(&content)
    }

    fn parse(content: &str) -> anyhow::Result<Self> {
        let config: Config = toml::from_str(content)?;

        anyhow::ensure!(
            !config.zones.is_empty(),
            "At least one zone must be configured"
        );

        // Check for duplicate IDs and GPIO collisions across zones, meters
        // and switches.
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
        for m in &config.meters {
            anyhow::ensure!(seen_ids.insert(&m.id), "Duplicate id: {} (meter)", m.id);
            anyhow::ensure!(
                seen_gpios.insert(m.gpio),
                "Duplicate GPIO {} (meter '{}')",
                m.gpio,
                m.id
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
            if let Some(driver) = &d.driver {
                anyhow::ensure!(
                    matches!(driver.as_str(), "ssd1306" | "ssd1309"),
                    "display.driver must be \"ssd1306\" or \"ssd1309\", got \"{}\"",
                    driver
                );
            }
            if let Some(gpio) = d.reset_gpio {
                anyhow::ensure!(
                    d.driver() == DisplayDriver::Ssd1309,
                    "display.reset_gpio requires driver = \"ssd1309\""
                );
                anyhow::ensure!(
                    seen_gpios.insert(gpio),
                    "Duplicate GPIO {} (display reset)",
                    gpio
                );
            }
        }

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"
[server]
port = 8090
[ha]
url = "http://ha"
token = "t"
[sprinkler]
[[zones]]
id = "z1"
name = "Z1"
gpio = 5
"#;

    #[test]
    fn unknown_display_driver_is_rejected() {
        let toml = format!("{BASE}[display]\ndriver = \"sh1106\"\n");
        let err = Config::parse(&toml).unwrap_err().to_string();
        assert!(err.contains("display.driver"), "{err}");
    }

    #[test]
    fn reset_gpio_requires_ssd1309() {
        let toml = format!("{BASE}[display]\nreset_gpio = 4\n");
        let err = Config::parse(&toml).unwrap_err().to_string();
        assert!(err.contains("reset_gpio requires"), "{err}");
    }

    #[test]
    fn reset_gpio_collision_is_rejected() {
        let toml = format!("{BASE}[display]\ndriver = \"ssd1309\"\nreset_gpio = 5\n");
        let err = Config::parse(&toml).unwrap_err().to_string();
        assert!(err.contains("Duplicate GPIO 5"), "{err}");
    }

    #[test]
    fn valid_ssd1309_config_parses() {
        let toml = format!("{BASE}[display]\ndriver = \"ssd1309\"\nreset_gpio = 4\ncontrast = 255\n");
        let config = Config::parse(&toml).unwrap();
        let d = config.display.unwrap();
        assert_eq!(d.driver(), DisplayDriver::Ssd1309);
        assert_eq!(d.reset_gpio, Some(4));
        assert_eq!(d.contrast, Some(255));
    }
}

impl ZoneConfig {
    pub fn max_duration(&self, default_secs: u64) -> Duration {
        Duration::from_secs(self.max_duration_secs.unwrap_or(default_secs))
    }
}
