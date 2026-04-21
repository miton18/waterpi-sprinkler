"""Sensor platform for WaterPi Sprinkler.

Exposes a ``last_opened_at`` timestamp sensor per zone.  The timestamp is
stored on the daemon side so it stays accurate even when other clients
(curl, automations) operate the valves directly.
"""

from __future__ import annotations

import asyncio
import logging
from datetime import datetime, timedelta

from homeassistant.components.sensor import SensorDeviceClass, SensorEntity
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers.aiohttp_client import async_get_clientsession
from homeassistant.helpers.device_registry import DeviceInfo
from homeassistant.helpers.entity_platform import AddEntitiesCallback
from homeassistant.helpers.typing import ConfigType, DiscoveryInfoType

from .const import DOMAIN, EVENT_STATE_UPDATE, SCAN_INTERVAL_SECS

_LOGGER = logging.getLogger(__name__)

SCAN_INTERVAL = timedelta(seconds=SCAN_INTERVAL_SECS)


async def async_setup_platform(
    hass: HomeAssistant,
    config: ConfigType,
    async_add_entities: AddEntitiesCallback,
    discovery_info: DiscoveryInfoType | None = None,
) -> None:
    """Create a last-opened timestamp sensor for each zone."""
    base_url: str = hass.data[DOMAIN]["base_url"]
    session = async_get_clientsession(hass)

    # Infinite retry with backoff (same logic as valve platform)
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

    entities: list[WaterpiLastOpenedSensor] = [
        WaterpiLastOpenedSensor(hass, base_url, zone) for zone in zones
    ]
    async_add_entities(entities, update_before_add=True)

    @callback
    def _handle_push(event):
        data = event.data
        zone_id = data.get("id")
        if zone_id is None:
            return
        for entity in entities:
            if entity.zone_id == zone_id:
                entity.apply_state(data)
                entity.async_write_ha_state()
                break

    hass.bus.async_listen(EVENT_STATE_UPDATE, _handle_push)


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
