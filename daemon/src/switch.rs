//! Physical toggle switches wired to GPIO inputs.
//!
//! Each switch sits between its GPIO line and GND, with the internal pull-up
//! enabled: contact closed = line LOW = ON (flip with `inverted`). Edges are
//! debounced by the kernel (gpio-cdev, Linux >= 5.10), so the interrupt only
//! fires on settled levels — the edge direction IS the stable state. Every
//! settled edge fires a `waterpi_switch_update` event on the Home Assistant
//! event bus; the REST endpoints read the pin level live, so polling always
//! reflects reality.

use std::sync::Arc;
use std::time::Duration;

use rppal::gpio::{Event, Gpio, InputPin, Trigger};
use serde::Serialize;
use tokio::runtime::Handle;
use tracing::info;

use crate::config::Config;
use crate::ha::HaClient;

pub type Switches = Arc<SwitchesInner>;

/// Mechanical toggle contact bounce is typically 5-20 ms; 50 ms gives ample
/// margin while staying far below any human-speed flip.
pub const DEFAULT_DEBOUNCE_MS: u64 = 50;

pub struct SwitchesInner {
    switches: Vec<Switch>,
}

struct Switch {
    id: String,
    name: String,
    gpio: u8,
    inverted: bool,
    debounce_ms: u64,
    // Kept alive: dropping the pin unregisters the async interrupt.
    pin: InputPin,
}

#[derive(Debug, Clone, Serialize)]
pub struct SwitchStatus {
    pub id: String,
    pub name: String,
    pub gpio: u8,
    pub is_on: bool,
    pub inverted: bool,
    pub debounce_ms: u64,
}

pub fn create(config: &Config, ha_client: HaClient) -> anyhow::Result<Switches> {
    let gpio = Gpio::new()?;
    let runtime = Handle::current();
    let mut switches = Vec::with_capacity(config.switches.len());

    for sc in &config.switches {
        let inverted = sc.inverted.unwrap_or(false);
        let debounce_ms = sc.debounce_ms.unwrap_or(DEFAULT_DEBOUNCE_MS);
        let debounce = (debounce_ms > 0).then(|| Duration::from_millis(debounce_ms));

        let mut pin = gpio.get(sc.gpio)?.into_input_pullup();

        let id = sc.id.clone();
        let ha = ha_client.clone();
        let rt = runtime.clone();
        pin.set_async_interrupt(Trigger::Both, debounce, move |event: Event| {
            // FallingEdge = line pulled LOW = contact closed.
            let is_on = matches!(event.trigger, Trigger::FallingEdge) != inverted;
            let ha = ha.clone();
            let id = id.clone();
            rt.spawn(async move {
                ha.push_switch_state(&id, is_on).await;
            });
        })?;

        let switch = Switch {
            id: sc.id.clone(),
            name: sc.name.clone(),
            gpio: sc.gpio,
            inverted,
            debounce_ms,
            pin,
        };
        info!(
            switch = %switch.id,
            gpio = switch.gpio,
            is_on = switch.is_on(),
            "Switch initialized"
        );
        switches.push(switch);
    }

    Ok(Arc::new(SwitchesInner { switches }))
}

pub fn get_all(switches: &Switches) -> Vec<SwitchStatus> {
    switches.switches.iter().map(Switch::status).collect()
}

pub fn get_switch(switches: &Switches, id: &str) -> Result<SwitchStatus, String> {
    switches
        .switches
        .iter()
        .find(|s| s.id == id)
        .map(Switch::status)
        .ok_or_else(|| format!("Switch '{}' not found", id))
}

impl Switch {
    fn is_on(&self) -> bool {
        self.pin.is_low() != self.inverted
    }

    fn status(&self) -> SwitchStatus {
        SwitchStatus {
            id: self.id.clone(),
            name: self.name.clone(),
            gpio: self.gpio,
            is_on: self.is_on(),
            inverted: self.inverted,
            debounce_ms: self.debounce_ms,
        }
    }
}
