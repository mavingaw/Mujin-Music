"""Config flow: manual and zeroconf discovery, pairing, password, reauth."""

from __future__ import annotations

from ipaddress import ip_address

import pytest
from pytest_homeassistant_custom_component.common import MockConfigEntry

from custom_components.mujin_music import config_flow

from custom_components.mujin_music.const import (
    CONF_ACCESS_TOKEN,
    CONF_REFRESH_TOKEN,
    DOMAIN,
)
from homeassistant.config_entries import SOURCE_USER, SOURCE_ZEROCONF
from homeassistant.const import CONF_HOST, CONF_PASSWORD, CONF_PORT, CONF_USERNAME
from homeassistant.core import HomeAssistant
from homeassistant.data_entry_flow import FlowResultType
from homeassistant.helpers.service_info.zeroconf import ZeroconfServiceInfo

from .fake_server import INSTANCE_ID, FakeMujinServer


def discovery(port: int, **properties: str) -> ZeroconfServiceInfo:
    return ZeroconfServiceInfo(
        ip_address=ip_address("127.0.0.1"),
        ip_addresses=[ip_address("127.0.0.1")],
        hostname="parson-8f6c1d0e.local.",
        name="Home Library._parson._tcp.local.",
        port=port,
        type="_parson._tcp.local.",
        properties={
            "id": INSTANCE_ID,
            "name": "Home Library",
            "product": "parson-music",
            **properties,
        },
    )


async def test_manual_setup_with_password(
    hass: HomeAssistant, fake_server: FakeMujinServer
) -> None:
    result = await hass.config_entries.flow.async_init(
        DOMAIN, context={"source": SOURCE_USER}
    )
    assert result["type"] is FlowResultType.FORM
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {CONF_HOST: "127.0.0.1", CONF_PORT: fake_server.port}
    )
    assert result["type"] is FlowResultType.MENU
    assert result["menu_options"] == ["pair", "login"]
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"next_step_id": "login"}
    )
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {CONF_USERNAME: "listener", CONF_PASSWORD: "wrong"}
    )
    assert result["errors"] == {"base": "invalid_auth"}
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"],
        {CONF_USERNAME: "listener", CONF_PASSWORD: "correct horse"},
    )
    assert result["type"] is FlowResultType.CREATE_ENTRY
    assert result["title"] == "Home Library"
    assert result["result"].unique_id == INSTANCE_ID
    # Tokens, never the password.
    assert CONF_PASSWORD not in result["data"]
    assert result["data"][CONF_ACCESS_TOKEN] == fake_server.access_token
    assert result["data"][CONF_REFRESH_TOKEN] == fake_server.refresh_token


async def test_manual_setup_unreachable(hass: HomeAssistant, socket_enabled: None) -> None:
    result = await hass.config_entries.flow.async_init(
        DOMAIN, context={"source": SOURCE_USER}
    )
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {CONF_HOST: "127.0.0.1", CONF_PORT: 9}
    )
    assert result["type"] is FlowResultType.FORM
    assert result["errors"] == {"base": "cannot_connect"}


async def test_zeroconf_discovery_and_pairing(
    hass: HomeAssistant, fake_server: FakeMujinServer, monkeypatch: pytest.MonkeyPatch
) -> None:
    result = await hass.config_entries.flow.async_init(
        DOMAIN,
        context={"source": SOURCE_ZEROCONF},
        data=discovery(fake_server.port),
    )
    assert result["type"] is FlowResultType.FORM
    assert result["step_id"] == "zeroconf_confirm"
    assert result["description_placeholders"]["name"] == "Home Library"
    result = await hass.config_entries.flow.async_configure(result["flow_id"], {})
    assert result["type"] is FlowResultType.MENU
    result = await hass.config_entries.flow.async_configure(
        result["flow_id"], {"next_step_id": "pair"}
    )
    assert result["step_id"] == "pair"
    assert result["description_placeholders"]["code"] == "482 913"

    # Submitting before approving in the app says so, keeps the same code.
    monkeypatch.setattr(config_flow, "PAIRING_WAIT_SECONDS", 0)
    result = await hass.config_entries.flow.async_configure(result["flow_id"], {})
    assert result["errors"] == {"base": "pairing_pending"}
    assert result["description_placeholders"]["code"] == "482 913"

    fake_server.pairing_approved = True
    result = await hass.config_entries.flow.async_configure(result["flow_id"], {})
    assert result["type"] is FlowResultType.CREATE_ENTRY
    assert result["data"][CONF_HOST] == "127.0.0.1"
    assert result["data"][CONF_ACCESS_TOKEN] == fake_server.access_token


async def test_zeroconf_follows_a_moved_server_and_ignores_others(
    hass: HomeAssistant, fake_server: FakeMujinServer
) -> None:
    entry = MockConfigEntry(
        domain=DOMAIN,
        unique_id=INSTANCE_ID,
        data={CONF_HOST: "10.0.0.9", CONF_PORT: 1993},
    )
    entry.add_to_hass(hass)
    result = await hass.config_entries.flow.async_init(
        DOMAIN,
        context={"source": SOURCE_ZEROCONF},
        data=discovery(fake_server.port),
    )
    assert result["type"] is FlowResultType.ABORT
    assert result["reason"] == "already_configured"
    assert entry.data[CONF_HOST] == "127.0.0.1"
    assert entry.data[CONF_PORT] == fake_server.port

    other = await hass.config_entries.flow.async_init(
        DOMAIN,
        context={"source": SOURCE_ZEROCONF},
        data=discovery(fake_server.port, product="something-else"),
    )
    assert other["reason"] == "not_mujin_music"


async def test_reauth_with_pairing_updates_tokens(
    hass: HomeAssistant, fake_server: FakeMujinServer, config_entry: MockConfigEntry
) -> None:
    config_entry.add_to_hass(hass)
    fake_server.issue_tokens()  # The saved session was revoked.
    fake_server.refresh_token = "revoked"
    assert not await hass.config_entries.async_setup(config_entry.entry_id)
    await hass.async_block_till_done()
    flows = hass.config_entries.flow.async_progress_by_handler(DOMAIN)
    assert [flow["context"]["source"] for flow in flows] == ["reauth"]
    flow_id = flows[0]["flow_id"]
    result = await hass.config_entries.flow.async_configure(flow_id, {})
    assert result["type"] is FlowResultType.MENU
    result = await hass.config_entries.flow.async_configure(
        flow_id, {"next_step_id": "pair"}
    )
    fake_server.pairing_approved = True
    result = await hass.config_entries.flow.async_configure(flow_id, {})
    assert result["type"] is FlowResultType.ABORT
    assert result["reason"] == "reauth_successful"
    assert config_entry.data[CONF_ACCESS_TOKEN] == fake_server.access_token
    await hass.async_block_till_done()
    assert await hass.config_entries.async_unload(config_entry.entry_id)
