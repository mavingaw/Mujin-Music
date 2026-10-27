"""Fixtures: a fake Mujin Music server and a config entry pointing at it."""

from __future__ import annotations

from collections.abc import AsyncGenerator

import pytest
from pytest_homeassistant_custom_component.common import MockConfigEntry

from custom_components.mujin_music.const import (
    CONF_ACCESS_TOKEN,
    CONF_INSTANCE_ID,
    CONF_REFRESH_TOKEN,
    CONF_USE_SSL,
    DOMAIN,
)
from homeassistant.const import CONF_HOST, CONF_PORT
from homeassistant.core import HomeAssistant

from .fake_server import INSTANCE_ID, FakeMujinServer, settle


@pytest.fixture(autouse=True)
def auto_enable_custom_integrations(enable_custom_integrations: None) -> None:
    """Loads custom_components/mujin_music in every test."""


@pytest.fixture
async def fake_server(socket_enabled: None) -> AsyncGenerator[FakeMujinServer]:
    server = FakeMujinServer()
    await server.start()
    yield server
    await server.close()


@pytest.fixture
def config_entry(fake_server: FakeMujinServer) -> MockConfigEntry:
    return MockConfigEntry(
        domain=DOMAIN,
        title="Home Library",
        unique_id=INSTANCE_ID,
        data={
            CONF_HOST: "127.0.0.1",
            CONF_PORT: fake_server.port,
            CONF_USE_SSL: False,
            CONF_INSTANCE_ID: INSTANCE_ID,
            CONF_ACCESS_TOKEN: fake_server.access_token,
            CONF_REFRESH_TOKEN: fake_server.refresh_token,
        },
    )


@pytest.fixture
async def loaded_entry(
    hass: HomeAssistant, fake_server: FakeMujinServer, config_entry: MockConfigEntry
) -> AsyncGenerator[MockConfigEntry]:
    config_entry.add_to_hass(hass)
    assert await hass.config_entries.async_setup(config_entry.entry_id)
    await settle(hass)
    yield config_entry
    assert await hass.config_entries.async_unload(config_entry.entry_id)
    await hass.async_block_till_done()
