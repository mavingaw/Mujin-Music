"""Mujin Music for Home Assistant: every zone as a media player."""

from __future__ import annotations

import aiohttp

from homeassistant.const import CONF_HOST, CONF_PORT, Platform
from homeassistant.core import HomeAssistant, ServiceCall, SupportsResponse
from homeassistant.exceptions import HomeAssistantError, ServiceValidationError
from homeassistant.helpers import config_validation as cv
from homeassistant.helpers.aiohttp_client import async_create_clientsession
from homeassistant.helpers.typing import ConfigType
import voluptuous as vol

from .api import MujinMusicClient, MujinMusicError, Tokens, server_url
from .const import (
    CONF_ACCESS_TOKEN,
    CONF_REFRESH_TOKEN,
    CONF_USE_SSL,
    DOMAIN,
    SERVICE_PAUSE_ALL,
)
from .coordinator import MujinMusicConfigEntry, MujinMusicCoordinator

PLATFORMS: list[Platform] = [Platform.MEDIA_PLAYER]

CONFIG_SCHEMA = cv.config_entry_only_config_schema(DOMAIN)


async def async_setup(hass: HomeAssistant, config: ConfigType) -> bool:
    """Registers the server-wide services."""

    async def pause_all(call: ServiceCall) -> dict[str, int]:
        entries = [
            entry
            for entry in hass.config_entries.async_loaded_entries(DOMAIN)
            if not call.data.get("config_entry_id")
            or entry.entry_id == call.data["config_entry_id"]
        ]
        if not entries:
            raise ServiceValidationError(
                translation_domain=DOMAIN, translation_key="no_servers"
            )
        paused = 0
        for entry in entries:
            coordinator: MujinMusicCoordinator = entry.runtime_data
            try:
                paused += await coordinator.client.pause_all()
            except MujinMusicError as error:
                raise HomeAssistantError(
                    translation_domain=DOMAIN,
                    translation_key="command_failed",
                    translation_placeholders={"error": str(error)},
                ) from error
            await coordinator.async_request_refresh()
        return {"paused": paused}

    hass.services.async_register(
        DOMAIN,
        SERVICE_PAUSE_ALL,
        pause_all,
        schema=vol.Schema({vol.Optional("config_entry_id"): cv.string}),
        supports_response=SupportsResponse.OPTIONAL,
    )
    return True


async def async_setup_entry(hass: HomeAssistant, entry: MujinMusicConfigEntry) -> bool:
    """Connects to one server and starts listening for zone events."""
    # The server sets session cookies on sign-in; this integration speaks
    # bearer tokens only, so cookies are never stored or sent.
    session = async_create_clientsession(hass, cookie_jar=aiohttp.DummyCookieJar())

    async def save_tokens(tokens: Tokens) -> None:
        hass.config_entries.async_update_entry(
            entry,
            data={
                **entry.data,
                CONF_ACCESS_TOKEN: tokens.access_token,
                CONF_REFRESH_TOKEN: tokens.refresh_token,
            },
        )

    client = MujinMusicClient(
        session,
        server_url(
            entry.data[CONF_HOST], entry.data[CONF_PORT], entry.data.get(CONF_USE_SSL, False)
        ),
        Tokens(entry.data[CONF_ACCESS_TOKEN], entry.data[CONF_REFRESH_TOKEN]),
        on_tokens=save_tokens,
    )
    coordinator = MujinMusicCoordinator(hass, entry, client, entry.title)
    await coordinator.async_config_entry_first_refresh()
    entry.runtime_data = coordinator
    await hass.config_entries.async_forward_entry_setups(entry, PLATFORMS)
    coordinator.start_events()
    return True


async def async_unload_entry(hass: HomeAssistant, entry: MujinMusicConfigEntry) -> bool:
    """Stops the event stream and removes the media players."""
    await entry.runtime_data.stop_events()
    return await hass.config_entries.async_unload_platforms(entry, PLATFORMS)
