"""Valve platform for WaterPi Sprinkler.

Each zone configured in the Rust daemon is exposed as a native HA
``valve`` entity with device class ``water``.  State is refreshed by
polling (every SCAN_INTERVAL seconds) *and* by listening for push
events fired by the daemon (``waterpi_sprinkler_update``).
"""

from __future__ import annotations

import asyncio
import logging
from datetime import timedelta

from homeassistant.components.valve import (
    ValveDeviceClass,
    ValveEntity,
    ValveEntityFeature,
)
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers.aiohttp_client import async_get_clientsession
from homeassistant.helpers.device_registry import DeviceInfo
from homeassistant.helpers.entity_platform import AddEntitiesCallback
from homeassistant.helpers.typing import ConfigType, DiscoveryInfoType

from .const import DOMAIN, EVENT_STATE_UPDATE, KIND_ICONS, SCAN_INTERVAL_SECS

_LOGGER = logging.getLogger(__name__)

SCAN_INTERVAL = timedelta(seconds=SCAN_INTERVAL_SECS)


async def async_setup_platform(
    hass: HomeAssistant,
    config: ConfigType,
    async_add_entities: AddEntitiesCallback,
    discovery_info: DiscoveryInfoType | None = None,
) -> None:
    """Discover zones from the daemon and create valve entities."""
    base_url: str = hass.data[DOMAIN]["base_url"]
    session = async_get_clientsession(hass)

    # Fetch initial zone list from daemon (infinite retry with backoff, max 60s)
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

    entities: list[WaterpiValve] = [
        WaterpiValve(hass, base_url, zone) for zone in zones
    ]
    async_add_entities(entities, update_before_add=True)

    # ── Push listener: daemon fires this event on every state change ──
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
    _LOGGER.info("Registered %d sprinkler valve(s)", len(entities))


# ---------------------------------------------------------------------------
# Entity
# ---------------------------------------------------------------------------


class WaterpiValve(ValveEntity):
    """Representation of a single irrigation zone / valve."""

    _attr_device_class = ValveDeviceClass.WATER
    _attr_supported_features = ValveEntityFeature.OPEN | ValveEntityFeature.CLOSE
    _attr_reports_position = False
    _attr_has_entity_name = True
    _attr_should_poll = True

    def __init__(self, hass: HomeAssistant, base_url: str, data: dict) -> None:
        self._base_url = base_url
        self._zone_id: str = data["id"]
        self._attr_name = data["name"]
        self._attr_unique_id = f"waterpi_{data['id']}"
        self._attr_is_closed = not data.get("is_open", False)
        self._kind: str | None = data.get("kind")
        self._attr_icon = KIND_ICONS.get(self._kind, None)
        self._extra: dict = data
        self.hass = hass

    # -- properties ----------------------------------------------------------

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

    @property
    def extra_state_attributes(self) -> dict:
        return {
            "gpio": self._extra.get("gpio"),
            "max_duration_secs": self._extra.get("max_duration_secs"),
            "open_duration_secs": self._extra.get("open_duration_secs"),
            "opened_at": self._extra.get("opened_at"),
        }

    # -- commands ------------------------------------------------------------

    async def async_open_valve(self, **kwargs) -> None:
        session = async_get_clientsession(self.hass)
        try:
            async with session.post(
                f"{self._base_url}/api/zones/{self._zone_id}/open", timeout=10
            ) as resp:
                if resp.status == 200:
                    self.apply_state(await resp.json())
        except Exception:
            _LOGGER.error("Failed to open valve %s", self._zone_id)

    async def async_close_valve(self, **kwargs) -> None:
        session = async_get_clientsession(self.hass)
        try:
            async with session.post(
                f"{self._base_url}/api/zones/{self._zone_id}/close", timeout=10
            ) as resp:
                if resp.status == 200:
                    self.apply_state(await resp.json())
        except Exception:
            _LOGGER.error("Failed to close valve %s", self._zone_id)

    # -- polling -------------------------------------------------------------

    async def async_update(self) -> None:
        session = async_get_clientsession(self.hass)
        try:
            async with session.get(
                f"{self._base_url}/api/zones/{self._zone_id}", timeout=10
            ) as resp:
                if resp.status == 200:
                    self.apply_state(await resp.json())
        except Exception:
            _LOGGER.warning("Failed to poll valve %s", self._zone_id)

    # -- state helpers -------------------------------------------------------

    @callback
    def apply_state(self, data: dict) -> None:
        """Apply state from daemon JSON payload (used by poll & push)."""
        self._attr_is_closed = not data.get("is_open", False)
        self._extra = data
