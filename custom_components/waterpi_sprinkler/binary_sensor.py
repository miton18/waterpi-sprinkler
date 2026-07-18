"""Binary sensor platform for WaterPi Sprinkler.

Exposes the physical toggle switches wired to the daemon's GPIO inputs as
read-only ``binary_sensor`` entities.  State is refreshed by polling (every
SCAN_INTERVAL seconds) *and* by listening for push events fired by the
daemon on every settled flip (``waterpi_switch_update``).
"""

from __future__ import annotations

import asyncio
import logging
from datetime import timedelta

from homeassistant.components.binary_sensor import BinarySensorEntity
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers.aiohttp_client import async_get_clientsession
from homeassistant.helpers.device_registry import DeviceInfo
from homeassistant.helpers.entity_platform import AddEntitiesCallback
from homeassistant.helpers.typing import ConfigType, DiscoveryInfoType

from .const import DOMAIN, EVENT_SWITCH_UPDATE, SCAN_INTERVAL_SECS

_LOGGER = logging.getLogger(__name__)

SCAN_INTERVAL = timedelta(seconds=SCAN_INTERVAL_SECS)


async def async_setup_platform(
    hass: HomeAssistant,
    config: ConfigType,
    async_add_entities: AddEntitiesCallback,
    discovery_info: DiscoveryInfoType | None = None,
) -> None:
    """Discover physical switches from the daemon and create entities."""
    base_url: str = hass.data[DOMAIN]["base_url"]
    session = async_get_clientsession(hass)

    # Fetch switch list from daemon (infinite retry with backoff, max 60s)
    switches = None
    attempt = 0
    delay = 2
    while switches is None:
        attempt += 1
        try:
            async with session.get(f"{base_url}/api/switches", timeout=10) as resp:
                if resp.status == 404:
                    # Older daemon without switch support.
                    _LOGGER.info(
                        "Daemon at %s has no /api/switches endpoint; no switch entities",
                        base_url,
                    )
                    return
                resp.raise_for_status()
                switches = await resp.json()
        except Exception:
            _LOGGER.warning(
                "Cannot reach waterpi-sprinkler daemon at %s (attempt %d), retrying in %ds",
                base_url, attempt, delay,
            )
            await asyncio.sleep(delay)
            delay = min(delay * 2, 60)

    if not switches:
        return

    entities: list[WaterpiSwitchSensor] = [
        WaterpiSwitchSensor(hass, base_url, sw) for sw in switches
    ]
    async_add_entities(entities, update_before_add=True)

    # ── Push listener: daemon fires this event on every settled flip ──
    @callback
    def _handle_push(event):
        data = event.data
        switch_id = data.get("id")
        if switch_id is None:
            return
        for entity in entities:
            if entity.switch_id == switch_id:
                entity.apply_state(data)
                entity.async_write_ha_state()
                break

    hass.bus.async_listen(EVENT_SWITCH_UPDATE, _handle_push)
    _LOGGER.info("Registered %d physical switch(es)", len(entities))


# ---------------------------------------------------------------------------
# Entity
# ---------------------------------------------------------------------------


class WaterpiSwitchSensor(BinarySensorEntity):
    """Representation of a physical toggle switch (read-only input)."""

    _attr_has_entity_name = True
    _attr_should_poll = True

    def __init__(self, hass: HomeAssistant, base_url: str, data: dict) -> None:
        self._base_url = base_url
        self._switch_id: str = data["id"]
        self._attr_name = data["name"]
        self._attr_unique_id = f"waterpi_switch_{data['id']}"
        self._attr_is_on = data.get("is_on", False)
        self._extra: dict = data
        self.hass = hass

    # -- properties ----------------------------------------------------------

    @property
    def switch_id(self) -> str:
        return self._switch_id

    @property
    def icon(self) -> str:
        return "mdi:toggle-switch" if self.is_on else "mdi:toggle-switch-off"

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
        return {"gpio": self._extra.get("gpio")}

    # -- polling -------------------------------------------------------------

    async def async_update(self) -> None:
        session = async_get_clientsession(self.hass)
        try:
            async with session.get(
                f"{self._base_url}/api/switches/{self._switch_id}", timeout=10
            ) as resp:
                if resp.status == 200:
                    self.apply_state(await resp.json())
        except Exception:
            _LOGGER.warning("Failed to poll switch %s", self._switch_id)

    # -- state helpers -------------------------------------------------------

    @callback
    def apply_state(self, data: dict) -> None:
        """Apply state from daemon JSON payload (used by poll & push).

        Push payloads are minimal (``{id, is_on}``) while poll payloads are
        the full descriptor — only overwrite the stored attributes when the
        payload actually carries them, so ``gpio`` doesn't blank out on push.
        """
        self._attr_is_on = data.get("is_on", self._attr_is_on)
        if "gpio" in data:
            self._extra = data
