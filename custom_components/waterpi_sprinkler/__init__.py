"""WaterPi Sprinkler integration — controls GPIO-based irrigation valves via a
remote Rust daemon running on a Raspberry Pi."""

import logging

import homeassistant.helpers.config_validation as cv
import voluptuous as vol
from homeassistant.core import HomeAssistant
from homeassistant.helpers.discovery import async_load_platform
from homeassistant.helpers.typing import ConfigType

from .const import CONF_HOST, CONF_PORT, DEFAULT_PORT, DOMAIN

_LOGGER = logging.getLogger(__name__)

CONFIG_SCHEMA = vol.Schema(
    {
        DOMAIN: vol.Schema(
            {
                vol.Required(CONF_HOST): cv.string,
                vol.Optional(CONF_PORT, default=DEFAULT_PORT): cv.port,
            }
        )
    },
    extra=vol.ALLOW_EXTRA,
)


async def async_setup(hass: HomeAssistant, config: ConfigType) -> bool:
    """Set up the WaterPi Sprinkler component from YAML."""
    conf = config[DOMAIN]
    host = conf[CONF_HOST]
    port = conf[CONF_PORT]
    base_url = f"http://{host}:{port}"

    hass.data[DOMAIN] = {"base_url": base_url}

    _LOGGER.info("WaterPi Sprinkler configured at %s", base_url)

    hass.async_create_task(
        async_load_platform(hass, "valve", DOMAIN, {}, config)
    )

    return True
