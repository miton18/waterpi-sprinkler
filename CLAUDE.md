# waterpi-sprinkler

GPIO irrigation controller: a Rust daemon on a Raspberry Pi 3 (**waterpi**) exposes valves, a water meter, physical switches and an OLED display over a REST API; a Home Assistant custom component (on **pi5**, HA in Docker) turns them into native HA entities.

**Status (2026-07-20): deployed and running in production.** `master` == what runs on waterpi.

## Architecture

- `daemon/` — Rust (tokio + axum 0.8 + rppal 0.22 `hal`, edition 2024). REST on port 8090; pushes instant state changes to HA's event bus (`POST /api/events/{type}` with a long-lived token).
- `custom_components/waterpi_sprinkler/` — HA component v1.4.0, YAML config (`waterpi_sprinkler: {host, port}`), platforms `valve` + `sensor` + `binary_sensor`, poll 10 s + event push.

### Daemon modules

| Module | Role |
|---|---|
| `sprinkler.rs` | 4 valve zones (GPIO out 5/6/13/19, active-low relays). Mutex (one open), 30 min max auto-close, `last_opened_at` persisted in `StateDirectory` |
| `meter.rs` | Honeywell V200 water meter (GPIO in 17, pull-up, falling edge, 1 pulse = 1 L). Stateless: fires `waterpi_meter_pulse` events; HA accumulates (RestoreSensor) |
| `switch.rs` | Physical toggle switches (GPIO in 16/20/21/26 ↔ GND, pull-up, closed = ON). Kernel debounce 50 ms, `Trigger::Both`, fires `waterpi_switch_update {id, is_on}`. REST reads pins live |
| `display.rs` | SSD1306 0.96" 128x64 OLED, I2C bus 1 @ 0x3C. Adaptive: idle = weather (or clock), watering = zone + % + progress bar; switches footer. Best-effort: never kills the daemon, auto re-init if unplugged. Render via `spawn_blocking` |
| `weather.rs` | Polls an HA `weather.*` entity every 5 min, shared cache, French condition labels |
| `ha.rs` | `HaClient`: generic `fire_event`, `push_state`, `push_meter_pulse`, `push_switch_state`, `get_state` |
| `api.rs` | Sub-routers merged (different axum states): `/api/zones*`, `/api/meters*`, `/api/switches*`, `/api/health` |
| `config.rs` | TOML; validates unique ids + no GPIO collisions across zones/meters/switches |

## Infra & deployment

- **waterpi**: `ssh waterpi` **from WSL only** (Windows ssh times out). Root login. Binary `/opt/waterpi-sprinkler`, config `/opt/waterpi-sprinkler.toml` (real HA token — never print it). Service **`sprinkler.service`** (`/lib/systemd/system`, User=root, `ExecStopPost` gpioset safety net). arm64 userland, runs the 32-bit static musl binary.
- **Build**: `make build` in `daemon/` from WSL → `armv7-unknown-linux-musleabihf`.
- **Upgrade sequence**: scp binary to `/tmp` → backup config → `systemctl stop sprinkler` → mv binary → adapt config → `systemctl start` → `is-active` + `journalctl -u sprinkler`.
- **pi5** (HA): `ssh pi5` (user miton18, passwordless sudo), HA in Docker container `homeassistant`. Component at `/home/miton18/home-assistant/ha-config/custom_components/waterpi_sprinkler/` (root-owned → scp to /tmp + `sudo cp`), then `sudo docker restart homeassistant`.
- HA URL `https://home.collignon-ducret.fr`, weather entity `weather.limony`. Verify entities via HA REST from waterpi (token in daemon config).

## Verification

- WSL: `cargo check`, `cargo clippy -- -D warnings`, `cargo test` (unit tests in `weather.rs`/`display.rs`). Config validation runs before `Gpio::new()`, so bad TOMLs are testable off-Pi.
- HA custom component: no tests; `python3 -m py_compile custom_components/waterpi_sprinkler/*.py` via WSL.

## Gotchas

- rppal is in maintenance mode (since 2025-07); it's fine, don't chase it.
- Kernel debounce (gpio-cdev) needs Linux ≥ 5.10; `debounce_ms = 0` bypasses it.
- I2C: enabled on waterpi; `dtparam=i2c_arm_baudrate=400000` is in `/boot/firmware/config.txt` but only applies after a reboot (100 kHz until then — works, just slower flushes).
- Switches GPIO 16/20/21/26 are configured but **not yet physically wired** (read OFF). Wiring: physical pins 36/38/40/37 ↔ GND (pin 34/39), no external resistor.
- GPIO 20/21 double as I2S — conflict only if an audio HAT ever appears.
- reqwest capped at 0.13.1 by Rust 1.95; `^0.13` will pick newer on a toolchain upgrade.
- Repo docs (README) are in French; keep it that way.
