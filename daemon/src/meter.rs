//! Pulse-input meters (e.g. Honeywell V200 water meter).
//!
//! A meter watches a GPIO line configured as input + pull-up. Each falling
//! edge (with software debounce) fires a `waterpi_meter_pulse` event on the
//! Home Assistant event bus with the configured increment. The daemon keeps
//! **no local state**: HA's sensor is the source of truth.

use std::sync::Arc;
use std::time::Duration;

use rppal::gpio::{Gpio, InputPin, Trigger};
use serde::Serialize;
use tokio::runtime::Handle;
use tracing::info;

use crate::config::Config;
use crate::ha::HaClient;

pub type Meters = Arc<MetersInner>;

pub struct MetersInner {
    meters: Vec<Meter>,
}

struct Meter {
    descriptor: MeterDescriptor,
    // Kept alive: dropping the pin unregisters the async interrupt.
    _pin: InputPin,
}

#[derive(Debug, Clone, Serialize)]
pub struct MeterDescriptor {
    pub id: String,
    pub name: String,
    pub gpio: u8,
    pub unit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_class: Option<String>,
    pub increment_per_pulse: u64,
}

pub fn create(config: &Config, ha_client: HaClient) -> anyhow::Result<Meters> {
    let gpio = Gpio::new()?;
    let mut meters = Vec::with_capacity(config.meters.len());
    let runtime = Handle::current();

    for mc in &config.meters {
        let descriptor = MeterDescriptor {
            id: mc.id.clone(),
            name: mc.name.clone(),
            gpio: mc.gpio,
            unit: mc.unit.clone().unwrap_or_else(|| "L".to_string()),
            device_class: mc.device_class.clone(),
            increment_per_pulse: mc.increment_per_pulse.unwrap_or(1),
        };

        let mut pin = gpio.get(mc.gpio)?.into_input_pullup();
        let debounce = Duration::from_millis(mc.debounce_ms.unwrap_or(200));

        let id = descriptor.id.clone();
        let unit = descriptor.unit.clone();
        let inc = descriptor.increment_per_pulse;
        let ha = ha_client.clone();
        let rt = runtime.clone();

        pin.set_async_interrupt(Trigger::FallingEdge, Some(debounce), move |_event| {
            info!(meter = %id, increment = inc, unit = %unit, "Pulse");
            let ha = ha.clone();
            let id = id.clone();
            let unit = unit.clone();
            rt.spawn(async move {
                ha.push_meter_pulse(&id, inc, &unit).await;
            });
        })?;

        info!(
            meter = %mc.id,
            gpio = mc.gpio,
            unit = %descriptor.unit,
            increment = inc,
            "Meter initialized"
        );

        meters.push(Meter {
            descriptor,
            _pin: pin,
        });
    }

    Ok(Arc::new(MetersInner { meters }))
}

impl MetersInner {
    pub fn all_descriptors(&self) -> Vec<MeterDescriptor> {
        let mut out: Vec<MeterDescriptor> =
            self.meters.iter().map(|m| m.descriptor.clone()).collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn get(&self, id: &str) -> Option<MeterDescriptor> {
        self.meters
            .iter()
            .find(|m| m.descriptor.id == id)
            .map(|m| m.descriptor.clone())
    }
}
