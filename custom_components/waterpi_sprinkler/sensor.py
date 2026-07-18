"""Sensor platform for WaterPi Sprinkler.

Exposes two kinds of sensors:

* ``last_opened_at`` timestamp per valve zone — stored on the daemon so it
  stays accurate even when other clients (curl, automations) operate the
  valves directly.
* total-increasing pulse meters (e.g. the Honeywell V200 water meter). The
  daemon is **stateless** for meters: it fires a ``waterpi_meter_pulse``
  event on every pulse and HA accumulates the value here (persisted across
  HA restarts via ``RestoreEntity``).
"""

from __future__ import annotations

import asyncio
import logging
from datetime import datetime, timedelta

from homeassistant.components.sensor import (
    RestoreSensor,
    SensorDeviceClass,
    SensorEntity,
    SensorStateClass,
)
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers.aiohttp_client import async_get_clientsession
from homeassistant.helpers.device_registry import DeviceInfo
from homeassistant.helpers.entity_platform import AddEntitiesCallback
from homeassistant.helpers.typing import ConfigType, DiscoveryInfoType

from .const import DOMAIN, EVENT_METER_PULSE, EVENT_STATE_UPDATE, SCAN_INTERVAL_SECS

# Mapping daemon device_class strings → HA SensorDeviceClass values.
_METER_DEVICE_CLASS_MAP = {
    "water": SensorDeviceClass.WATER,
}

_LOGGER = logging.getLogger(__name__)

SCAN_INTERVAL = timedelta(seconds=SCAN_INTERVAL_SECS)


async def async_setup_platform(
    hass: HomeAssistant,
    config: ConfigType,
    async_add_entities: AddEntitiesCallback,
    discovery_info: DiscoveryInfoType | None = None,
) -> None:
    """Create last-opened sensors for valves and accumulating meter sensors."""
    base_url: str = hass.data[DOMAIN]["base_url"]
    session = async_get_clientsession(hass)

    # ── Valve "last opened" sensors ─────────────────────────────────────────
    zones = None
    attempt = 0
    delay = 2
    while zones is None:
        attempt += 1
        try:
            async with session.get(f"{base_url}/api/zones", timeout=10) as resp:
                resp.raise_for_status()
                zones = await resp.json()
        except Exception:
            _LOGGER.warning(
                "Cannot reach waterpi-sprinkler daemon at %s (attempt %d), retrying in %ds",
                base_url, attempt, delay,
            )
            await asyncio.sleep(delay)
            delay = min(delay * 2, 60)

    last_opened_entities: list[WaterpiLastOpenedSensor] = [
        WaterpiLastOpenedSensor(hass, base_url, zone) for zone in zones
    ]
    async_add_entities(last_opened_entities, update_before_add=True)

    @callback
    def _handle_push(event):
        data = event.data
        zone_id = data.get("id")
        if zone_id is None:
            return
        for entity in last_opened_entities:
            if entity.zone_id == zone_id:
                entity.apply_state(data)
                entity.async_write_ha_state()
                break

    hass.bus.async_listen(EVENT_STATE_UPDATE, _handle_push)

    # ── Meters (pulse accumulators) ─────────────────────────────────────────
    meters = await _fetch_meters(session, base_url)
    if not meters:
        return

    meter_entities = [WaterpiMeter(hass, m) for m in meters]
    async_add_entities(meter_entities)
    _LOGGER.info("Registered %d meter sensor(s)", len(meter_entities))


async def _fetch_meters(session, base_url: str) -> list[dict]:
    """Fetch meter descriptors from the daemon. Returns [] on failure."""
    try:
        async with session.get(f"{base_url}/api/meters", timeout=10) as resp:
            if resp.status == 404:
                # Older daemon without meter support.
                return []
            resp.raise_for_status()
            return await resp.json()
    except Exception as e:
        _LOGGER.warning("Failed to fetch meters from %s: %s", base_url, e)
        return []


# ── Valve "last opened" sensor ─────────────────────────────────────────────


class WaterpiLastOpenedSensor(SensorEntity):
    """Timestamp sensor showing the last time a zone was opened."""

    _attr_device_class = SensorDeviceClass.TIMESTAMP
    _attr_has_entity_name = True
    _attr_should_poll = True
    _attr_icon = "mdi:clock-check-outline"

    def __init__(self, hass: HomeAssistant, base_url: str, data: dict) -> None:
        self._base_url = base_url
        self._zone_id: str = data["id"]
        self._attr_name = f"{data['name']} dernier arrosage"
        self._attr_unique_id = f"waterpi_{data['id']}_last_opened"
        self._attr_native_value = _parse_dt(data.get("last_opened_at"))
        self.hass = hass

    @property
    def zone_id(self) -> str:
        return self._zone_id

    @property
    def device_info(self) -> DeviceInfo:
        return DeviceInfo(
            identifiers={(DOMAIN, "waterpi_sprinkler")},
            name="WaterPi Sprinkler",
            manufacturer="DIY",
            model="Raspberry Pi GPIO Sprinkler",
        )

    async def async_update(self) -> None:
        session = async_get_clientsession(self.hass)
        try:
            async with session.get(
                f"{self._base_url}/api/zones/{self._zone_id}", timeout=10
            ) as resp:
                if resp.status == 200:
                    self.apply_state(await resp.json())
        except Exception:
            _LOGGER.warning("Failed to poll last_opened sensor %s", self._zone_id)

    @callback
    def apply_state(self, data: dict) -> None:
        self._attr_native_value = _parse_dt(data.get("last_opened_at"))


def _parse_dt(value: str | None) -> datetime | None:
    if value is None:
        return None
    return datetime.fromisoformat(value)


# ── Meter (pulse accumulator) ──────────────────────────────────────────────


class WaterpiMeter(RestoreSensor):
    """A WaterPi pulse meter. Accumulates pulses from daemon events.

    The daemon keeps no state — every pulse fires a ``waterpi_meter_pulse``
    event with ``{ id, increment, unit }`` and this sensor adds the increment
    to its running total. The value is persisted by HA's standard restore
    mechanism, so it survives HA restarts. After a recalibration on the
    physical meter (or to bootstrap), set the entity state directly via the
    Developer Tools or the ``homeassistant.set_state`` service.
    """

    _attr_has_entity_name = True
    _attr_should_poll = False
    _attr_state_class = SensorStateClass.TOTAL_INCREASING
    _attr_icon = "mdi:water-pump"

    def __init__(self, hass: HomeAssistant, descriptor: dict) -> None:
        self.hass = hass
        self._meter_id: str = descriptor["id"]
        self._attr_name = descriptor["name"]
        self._attr_unique_id = f"waterpi_meter_{descriptor['id']}"
        self._attr_native_unit_of_measurement = descriptor.get("unit") or "L"
        raw_dc = descriptor.get("device_class")
        if raw_dc and raw_dc in _METER_DEVICE_CLASS_MAP:
            self._attr_device_class = _METER_DEVICE_CLASS_MAP[raw_dc]
        # Native value starts at 0 — will be overwritten by RestoreEntity on
        # async_added_to_hass if a previous state exists.
        self._attr_native_value = 0
        self._gpio = descriptor.get("gpio")

    @property
    def meter_id(self) -> str:
        return self._meter_id

    @property
    def device_info(self) -> DeviceInfo:
        return DeviceInfo(
            identifiers={(DOMAIN, "waterpi_sprinkler")},
            name="WaterPi Sprinkler",
            manufacturer="DIY",
            model="Raspberry Pi GPIO Sprinkler",
        )

    @property
    def extra_state_attributes(self) -> dict:
        return {"gpio": self._gpio}

    async def async_added_to_hass(self) -> None:
        """Restore previous value and subscribe to pulse events."""
        await super().async_added_to_hass()

        last = await self.async_get_last_sensor_data()
        if last is not None and last.native_value is not None:
            try:
                self._attr_native_value = int(last.native_value)
            except (TypeError, ValueError):
                _LOGGER.warning(
                    "Cannot restore previous value %r for meter %s — starting at 0",
                    last.native_value, self._meter_id,
                )

        self.async_on_remove(
            self.hass.bus.async_listen(EVENT_METER_PULSE, self._handle_pulse)
        )

    @callback
    def _handle_pulse(self, event) -> None:
        data = event.data
        if data.get("id") != self._meter_id:
            return
        increment = data.get("increment", 1)
        try:
            self._attr_native_value = int(self._attr_native_value or 0) + int(increment)
        except (TypeError, ValueError):
            _LOGGER.warning("Invalid increment %r for meter %s", increment, self._meter_id)
            return
        self.async_write_ha_state()
