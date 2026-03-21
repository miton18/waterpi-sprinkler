# Handover — waterpi-sprinkler

**Session date:** 2026-03-21
**Session summary:** Designed and coded a complete GPIO-based irrigation controller: a Rust daemon for Raspberry Pi (waterpi) + a Home Assistant custom component exposing native `valve` entities. Code is written but not yet compiled or tested.

---

## What We Were Working On

Rémi has a buried irrigation system with 4 electrovalves controlled by relays wired to a Raspberry Pi named **waterpi** (Pi 3, armv7). Home Assistant runs on a separate Raspberry Pi named **pi5**. He was using the `remote_rpi_gpio` integration (pigpio-based) to control the GPIOs remotely, but it broke (pigpio daemon issues, poorly maintained HA integration, no Pi 5 support).

The goal: replace this with a proper, self-contained solution that exposes **native `valve` entities** in HA (like ESPHome's `sprinkler` component does), while keeping the existing Raspberry Pi hardware.

## What Got Done

### 1. Rust daemon (`daemon/`)

A `tokio` + `axum` + `rppal` daemon that runs on waterpi and:

- Controls 4 GPIO pins (5, 6, 13, 19) via rppal, with **invert_logic = true** (active-low relay board: GPIO LOW = relay ON = valve open)
- Enforces a **mutex**: only one valve open at a time. Opening a new zone auto-closes any currently open zone.
- Enforces a **max duration** (30 min default, configurable per zone). A `tokio::spawn` timer auto-closes the valve and the handle is `.abort()`ed on manual close.
- Exposes a **REST API** on port 8090:
  - `GET /api/health` → `"ok"`
  - `GET /api/zones` → list all zones with state
  - `GET /api/zones/{id}` → single zone state
  - `POST /api/zones/{id}/open` → open valve
  - `POST /api/zones/{id}/close` → close valve
  - `POST /api/zones/close-all` → emergency close all
- **Pushes state changes to HA** by firing `waterpi_sprinkler_update` events via `POST /api/events/waterpi_sprinkler_update` on HA's REST API (requires a long-lived token).
- **Graceful shutdown**: SIGTERM/SIGINT handler closes all valves before exit.
- **Systemd safety net**: `ExecStopPost` runs `gpioset gpiochip0 5=1 6=1 13=1 19=1` to force all GPIOs HIGH (valves closed) if the daemon crashes.
- Configuration via TOML (`config.example.toml` provided with Rémi's exact zone mapping).

### 2. HA custom component (`custom_components/waterpi_sprinkler/`)

A minimal Python integration that:

- Creates 4 `ValveEntity` instances with `ValveDeviceClass.WATER`
- Grouped under a single HA device "WaterPi Sprinkler"
- Supports `valve.open` / `valve.close` services (maps to REST calls to the daemon)
- **Dual state refresh**: polls every 10s + listens to `waterpi_sprinkler_update` events on the HA event bus for instant updates (timeout, mutex close)
- Extra attributes: `gpio`, `max_duration_secs`, `open_duration_secs`, `opened_at`
- YAML config only (no config flow):
  ```yaml
  waterpi_sprinkler:
    host: waterpi
    port: 8090
  ```

### 3. Systemd service + README

Complete deployment docs in `README.md` including cross-compilation instructions.

## What Didn't Work (and How We Fixed It)

1. **rppal `gpio` feature flag**: rppal 0.19 includes GPIO by default; specifying `features = ["gpio"]` causes a build error. Fix: removed the feature flag from `Cargo.toml`.
2. **Rust edition**: originally set to `2021`, updated to `2024` (stable since Rust 1.85).

## Key Decisions & Rationale

| Decision | Rationale |
|----------|-----------|
| **Rust daemon + HA custom component** (option 2) over ESPHome protocol emulation or MQTT | Rémi doesn't use MQTT (explicit preference). ESPHome protobuf reverse engineering is fragile. A REST API + thin HA component is the cleanest separation of concerns. |
| **Native `valve` entities** (not `switch`) | Rémi explicitly wanted ESPHome-style sprinkler entities. `ValveEntity` with `ValveDeviceClass.WATER` gives proper icons, states, and semantics in HA. |
| **Push + poll** (option C) | Poll alone means delayed state on timeout/mutex events. Push alone couples the daemon to HA availability. Both gives best UX with minimal extra code (just a `reqwest` POST). |
| **invert_logic = true** | Rémi had `invert_logic: true` in his old `remote_rpi_gpio` config. Active-low relay boards: GPIO LOW = relay energized = valve open. |
| **axum** over warp | More actively maintained, Rémi knows both. |
| **rppal** over libgpiod bindings | Rémi already uses rppal in other projects. Direct memory-mapped GPIO, no dependency on libgpiod daemon. |
| **TOML config** | Consistent with Rust ecosystem conventions. |
| **Mutex at daemon level** (not HA level) | Safety logic must live close to the hardware. If HA sends two concurrent open commands, the daemon enforces the constraint regardless. |
| **Port 8090** | Rémi's choice. |

## Lessons Learned & Gotchas

- **rppal 0.19 features**: GPIO is in default features, don't add it explicitly.
- **`gpioset -m signal`**: needed to keep the line held after the process exits. Relevant for the systemd `ExecStopPost` fallback (which doesn't use `-m signal` because we want a one-shot set-and-release, but the GPIO state persists).
- **gpiochip numbering**: Pi 3 uses `gpiochip0`. Pi 5 uses `gpiochip4`. The daemon uses rppal which handles this, but the systemd `ExecStopPost` gpioset command hardcodes `gpiochip0` — must be verified on the actual waterpi hardware.
- **HA `valve` platform**: relatively new in HA (2023.11+). Uses `is_closed` (bool) not `is_on`. `ValveEntityFeature.OPEN | CLOSE` for basic on/off without position support.
- **Cross-compilation**: waterpi is a Pi 3 → target `armv7-unknown-linux-gnueabihf`, linker `arm-linux-gnueabihf-gcc`. Rémi builds from WSL.

## Next Steps

- [ ] **Compile the daemon** — `cargo build --release --target armv7-unknown-linux-gnueabihf` from WSL. Fix any remaining compile errors.
- [ ] **Test on waterpi** — deploy the binary, create `config.toml` with a real HA token, start manually, test with `curl`.
- [ ] **Verify gpiochip** — run `gpioinfo` on waterpi to confirm `gpiochip0` and that GPIOs 5/6/13/19 are the right lines.
- [ ] **Test HA integration** — copy custom component, add YAML config, restart HA, verify valve entities appear.
- [ ] **Test safety features** — open a valve, wait 30 min (or lower the timeout for testing), confirm auto-close. Open two valves, confirm mutex. Kill the daemon, confirm `ExecStopPost` fires.
- [ ] **Consider adding**: a `/api/zones/{id}/open?duration_secs=300` parameter to override duration per-call (useful for HA automations like "arrose 5 min zone 1 puis 5 min zone 2").
- [ ] **Consider adding**: a sequence endpoint (`POST /api/sequence` with a list of zone IDs + durations) for automated full-garden watering cycles.
- [ ] **Consider adding**: a config flow (UI-based setup) for the HA component instead of YAML-only.
- [ ] **Consider adding**: HA `number` entities to expose/adjust max_duration per zone from the HA UI.

## Important Files Map

| File | Role |
|------|------|
| `daemon/Cargo.toml` | Rust dependencies — edition 2024, axum 0.8, rppal 0.19, tokio, reqwest, etc. |
| `daemon/src/main.rs` | Entry point: loads config, inits sprinkler, starts axum server, graceful shutdown handler |
| `daemon/src/config.rs` | TOML config parsing + validation (ServerConfig, HaConfig, SprinklerConfig, ZoneConfig) |
| `daemon/src/sprinkler.rs` | Core logic: `Arc<Mutex<SprinklerInner>>` with zones, GPIO pins (rppal `OutputPin`), mutex enforcement, max duration timers, open/close/close_all operations |
| `daemon/src/api.rs` | Axum router: 6 REST endpoints mapping to sprinkler operations |
| `daemon/src/ha.rs` | HaClient: pushes `waterpi_sprinkler_update` events to HA REST API via reqwest |
| `daemon/config.example.toml` | Example config with Rémi's exact 4-zone GPIO mapping (5, 6, 13, 19) |
| `daemon/waterpi-sprinkler.service` | Systemd unit with ExecStopPost safety net |
| `custom_components/waterpi_sprinkler/__init__.py` | HA integration setup: YAML schema, discovery of valve platform |
| `custom_components/waterpi_sprinkler/valve.py` | ValveEntity subclass: open/close via REST, poll + push event listener, device grouping |
| `custom_components/waterpi_sprinkler/const.py` | Constants: domain, default port, scan interval, event name |
| `custom_components/waterpi_sprinkler/manifest.json` | HA integration manifest |
| `README.md` | Architecture diagram, setup instructions, API reference, cross-compilation guide |

## Current State

**Not yet compiled or tested.** The code was written in a claude.ai session without access to a Rust compiler or Raspberry Pi. Expected issues:

1. Possible minor Rust compile errors (edition 2024 may have stricter rules on some patterns)
2. The HA custom component Python code needs validation against the actual HA valve platform API (imports, method signatures)
3. GPIO pin mapping needs physical verification on waterpi
4. The `ExecStopPost` gpioset command assumes `gpiochip0` — to verify
5. No tests written yet
