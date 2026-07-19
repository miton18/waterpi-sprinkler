//! SSD1306 128x64 OLED status display (I2C).
//!
//! Adaptive layout: idle shows the weather (or a clock when no weather entity
//! is configured), watering shows the running zone with a progress bar. The
//! physical switch states sit in a fixed footer on both views.
//!
//! The display is strictly best-effort: init failure means the daemon runs
//! without it, and mid-run I2C errors are rate-limit logged with periodic
//! re-init attempts (recovers a re-plugged panel). Rendering and flushing go
//! through `spawn_blocking` so the ~25 ms I2C flush (400 kHz) never blocks
//! the runtime.

use std::time::Duration;

use chrono::Local;
use embedded_graphics::{
    mono_font::{
        MonoTextStyle,
        iso_8859_1::{FONT_6X10, FONT_7X13_BOLD, FONT_10X20},
    },
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::{Baseline, Text},
};
use rppal::i2c::I2c;
use ssd1306::{
    I2CDisplayInterface, Ssd1306,
    mode::{BufferedGraphicsMode, DisplayConfig as _},
    prelude::I2CInterface,
    rotation::DisplayRotation,
    size::DisplaySize128x64,
};
use tokio::sync::oneshot;
use tracing::{info, warn};

use crate::config::DisplayConfig as DisplayCfg;
use crate::sprinkler::{self, Sprinkler};
use crate::switch::{self, Switches};
use crate::weather::{self, WeatherCache};

type Oled = Ssd1306<I2CInterface<I2c>, DisplaySize128x64, BufferedGraphicsMode<DisplaySize128x64>>;

// Every N consecutive render failures, try a re-init (recovers a power-cycled
// panel); warn once at the first failure then every LOG_EVERY ticks.
const REINIT_EVERY: u32 = 30;
const LOG_EVERY: u32 = 60;

pub struct DisplayHandle {
    shutdown_tx: oneshot::Sender<()>,
    join: tokio::task::JoinHandle<()>,
}

impl DisplayHandle {
    /// Ask the render loop to stop, clear and switch off the panel.
    /// Bounded so shutdown can never hang on a wedged I2C bus.
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), self.join).await;
    }
}

/// Init the I2C bus + panel and spawn the render loop.
/// On Err the caller logs and continues without a display.
pub fn spawn(
    cfg: &DisplayCfg,
    sprinkler: Sprinkler,
    switches: Switches,
    weather: WeatherCache,
) -> anyhow::Result<DisplayHandle> {
    let i2c = I2c::with_bus(cfg.i2c_bus)?;
    let iface = I2CDisplayInterface::new_custom_address(i2c, cfg.address);
    let mut display = Ssd1306::new(iface, DisplaySize128x64, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    display
        .init()
        .map_err(|e| anyhow::anyhow!("SSD1306 init failed (bus {}, addr 0x{:02X}): {e:?}", cfg.i2c_bus, cfg.address))?;
    info!(bus = cfg.i2c_bus, address = format!("0x{:02X}", cfg.address), "OLED initialized");

    let refresh = Duration::from_secs(cfg.refresh_secs);
    let has_weather = cfg.weather_entity.is_some();
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();

    let join = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(refresh);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut display = Some(display);
        let mut consecutive_errors: u32 = 0;

        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                _ = ticker.tick() => {
                    let frame = build_frame(&sprinkler, &switches, &weather, has_weather).await;
                    let mut d = display.take().expect("display always restored");
                    let reinit =
                        consecutive_errors > 0 && consecutive_errors.is_multiple_of(REINIT_EVERY);
                    let (d, res) = tokio::task::spawn_blocking(move || {
                        if reinit {
                            let _ = d.init();
                        }
                        let res = render(&mut d, &frame);
                        (d, res)
                    })
                    .await
                    .expect("render task panicked");
                    display = Some(d);

                    match res {
                        Ok(()) => consecutive_errors = 0,
                        Err(e) => {
                            if consecutive_errors == 0 || consecutive_errors.is_multiple_of(LOG_EVERY) {
                                warn!(error = %e, "OLED render failed (panel unplugged?)");
                            }
                            consecutive_errors = consecutive_errors.saturating_add(1);
                        }
                    }
                }
            }
        }

        // Best-effort: blank and switch off the panel on shutdown.
        if let Some(mut d) = display.take() {
            let _ = tokio::task::spawn_blocking(move || {
                d.clear_buffer();
                let _ = d.flush();
                let _ = d.set_display_on(false);
            })
            .await;
        }
    });

    Ok(DisplayHandle { shutdown_tx, join })
}

// ---------------------------------------------------------------------------
// Frame snapshot (pure data, decouples async gathering from blocking drawing)
// ---------------------------------------------------------------------------

enum View {
    IdleClock {
        time: String,
    },
    IdleWeather {
        temperature: Option<f64>,
        condition: Option<String>,
    },
    Watering {
        zone_name: String,
        pct: u8,
        remaining_secs: u64,
    },
}

struct Frame {
    view: View,
    switches: Vec<(String, bool)>,
}

async fn build_frame(
    sprinkler: &Sprinkler,
    switches: &Switches,
    weather: &WeatherCache,
    has_weather: bool,
) -> Frame {
    let zones = sprinkler::get_all(sprinkler).await;
    let switch_states = switch::get_all(switches)
        .into_iter()
        .map(|s| (s.name, s.is_on))
        .collect();

    let view = if let Some(zone) = zones.iter().find(|z| z.is_open) {
        let elapsed = zone.open_duration_secs.unwrap_or(0);
        View::Watering {
            zone_name: zone.name.clone(),
            pct: watering_pct(elapsed, zone.max_duration_secs),
            remaining_secs: zone.max_duration_secs.saturating_sub(elapsed),
        }
    } else if has_weather {
        let cached = weather.read().unwrap().clone();
        View::IdleWeather {
            temperature: cached.as_ref().and_then(|w| w.temperature),
            condition: cached.map(|w| w.condition),
        }
    } else {
        View::IdleClock {
            time: Local::now().format("%H:%M").to_string(),
        }
    };

    Frame {
        view,
        switches: switch_states,
    }
}

fn watering_pct(elapsed: u64, max: u64) -> u8 {
    if max == 0 {
        return 100;
    }
    (elapsed * 100 / max).min(100) as u8
}

// ---------------------------------------------------------------------------
// Rendering (blocking; draws into the framebuffer then flushes over I2C)
// ---------------------------------------------------------------------------

/// Drawing into the framebuffer is infallible; only the I2C flush can fail.
fn render(display: &mut Oled, frame: &Frame) -> Result<(), String> {
    let small = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let bold = MonoTextStyle::new(&FONT_7X13_BOLD, BinaryColor::On);
    let big = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);
    let stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let fill = PrimitiveStyle::with_fill(BinaryColor::On);

    display.clear_buffer();

    match &frame.view {
        View::IdleClock { time } => {
            draw_centered(display, time, &big, 10, 11);
        }
        View::IdleWeather {
            temperature,
            condition,
        } => {
            let temp = match temperature {
                Some(t) => format!("{:.0}°", t),
                None => "--°".to_string(),
            };
            draw_centered(display, &temp, &big, 10, 2);
            if let Some(c) = condition {
                let label = truncate_chars(weather::condition_fr(c), 21);
                draw_centered(display, &label, &small, 6, 26);
            }
        }
        View::Watering {
            zone_name,
            pct,
            remaining_secs,
        } => {
            let name = truncate_chars(zone_name, 18);
            Text::with_baseline(&name, Point::new(0, 0), bold, Baseline::Top)
                .draw(display)
                .unwrap();
            let pct_text = format!("{}%", pct);
            Text::with_baseline(&pct_text, Point::new(0, 14), big, Baseline::Top)
                .draw(display)
                .unwrap();
            let remaining = fmt_remaining(*remaining_secs);
            Text::with_baseline(&remaining, Point::new(62, 19), small, Baseline::Top)
                .draw(display)
                .unwrap();
            Rectangle::new(Point::new(0, 35), Size::new(128, 6))
                .into_styled(stroke)
                .draw(display)
                .unwrap();
            let w = *pct as u32 * 124 / 100;
            if w > 0 {
                Rectangle::new(Point::new(2, 37), Size::new(w, 2))
                    .into_styled(fill)
                    .draw(display)
                    .unwrap();
            }
        }
    }

    // Fixed footer: physical switch states on both views.
    if !frame.switches.is_empty() {
        Line::new(Point::new(0, 42), Point::new(127, 42))
            .into_styled(stroke)
            .draw(display)
            .unwrap();
        for (i, line) in switch_lines(&frame.switches).iter().enumerate() {
            Text::with_baseline(line, Point::new(0, 44 + 10 * i as i32), small, Baseline::Top)
                .draw(display)
                .unwrap();
        }
    }

    display.flush().map_err(|e| format!("{e:?}"))
}

fn draw_centered(display: &mut Oled, text: &str, style: &MonoTextStyle<'_, BinaryColor>, char_w: usize, y: i32) {
    let width = text.chars().count() * char_w;
    let x = (128usize.saturating_sub(width) / 2) as i32;
    Text::with_baseline(text, Point::new(x, y), *style, Baseline::Top)
        .draw(display)
        .unwrap();
}

/// Two switches per 21-column line: `Name123 ON Name456 --`.
fn switch_lines(switches: &[(String, bool)]) -> Vec<String> {
    switches
        .chunks(2)
        .take(2)
        .map(|pair| {
            pair.iter()
                .map(|(name, is_on)| format!("{:<7.7}{:>3}", name, if *is_on { "ON" } else { "--" }))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

fn fmt_remaining(secs: u64) -> String {
    format!("reste {:02}:{:02}", secs / 60, secs % 60)
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pct_is_bounded() {
        assert_eq!(watering_pct(0, 1800), 0);
        assert_eq!(watering_pct(1206, 1800), 67);
        assert_eq!(watering_pct(1800, 1800), 100);
        assert_eq!(watering_pct(9999, 1800), 100);
        assert_eq!(watering_pct(5, 0), 100);
    }

    #[test]
    fn remaining_formats_mm_ss() {
        assert_eq!(fmt_remaining(252), "reste 04:12");
        assert_eq!(fmt_remaining(0), "reste 00:00");
        assert_eq!(fmt_remaining(3599), "reste 59:59");
    }

    #[test]
    fn switch_lines_fit_21_columns() {
        let switches = vec![
            ("Interrupteur 1".to_string(), true),
            ("I2".to_string(), false),
            ("Été".to_string(), true),
            ("I4".to_string(), false),
        ];
        let lines = switch_lines(&switches);
        assert_eq!(lines.len(), 2);
        for line in &lines {
            assert!(line.chars().count() <= 21, "line too long: {line:?}");
        }
        assert_eq!(lines[0], "Interru ON I2      --");
    }

    #[test]
    fn truncation_is_char_safe() {
        assert_eq!(truncate_chars("Ensoleillé", 21), "Ensoleillé");
        assert_eq!(truncate_chars("Partiellement nuageux et plus", 21), "Partiellement nuageux");
    }
}
