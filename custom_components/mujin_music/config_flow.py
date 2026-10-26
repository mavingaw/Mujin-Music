"""Config flow: find the server (zeroconf or by address), then sign in.

Signing in is either pairing (Home Assistant shows a six-digit code, you
approve it in the Mujin Music app, no password typed into Home Assistant)
or a username and password. Both end with a revocable device session.
"""

from __future__ import annotations

import asyncio
from collections.abc import Mapping
import logging
from typing import Any

import aiohttp
import voluptuous as vol

from homeassistant.config_entries import (
    SOURCE_REAUTH,
    ConfigFlow,
    ConfigFlowResult,
)
from homeassistant.const import CONF_HOST, CONF_PASSWORD, CONF_PORT, CONF_USERNAME
from homeassistant.helpers.aiohttp_client import async_create_clientsession
from homeassistant.helpers.service_info.zeroconf import ZeroconfServiceInfo

from .api import (
    MujinMusicAuthError,
    MujinMusicCommandError,
    MujinMusicConnectionError,
    Pairing,
    ServerInfo,
    Tokens,
    fetch_server_info,
    login,
    pairing_status,
    server_url,
    start_pairing,
)
from .const import (
    BRAND,
    CONF_ACCESS_TOKEN,
    CONF_INSTANCE_ID,
    CONF_REFRESH_TOKEN,
    CONF_USE_SSL,
    DEFAULT_PORT,
    DOMAIN,
    PRODUCT_ID,
)

_LOGGER = logging.getLogger(__name__)

PAIRING_DEVICE_NAME = "Home Assistant"
# After "Submit", wait this long for an approval that is on its way.
PAIRING_WAIT_SECONDS = 4.0
PAIRING_POLL_SECONDS = 0.5


class MujinMusicConfigFlow(ConfigFlow, domain=DOMAIN):
    """Adds a Mujin Music server."""

    VERSION = 1

    def __init__(self) -> None:
        self._host: str | None = None
        self._port: int = DEFAULT_PORT
        self._ssl = False
        self._info: ServerInfo | None = None
        self._pairing: Pairing | None = None
        self._session: aiohttp.ClientSession | None = None

    @property
    def _base_url(self) -> str:
        assert self._host is not None
        return server_url(self._host, self._port, self._ssl)

    def _client_session(self) -> aiohttp.ClientSession:
        if self._session is None:
            self._session = async_create_clientsession(
                self.hass, cookie_jar=aiohttp.DummyCookieJar()
            )
        return self._session

    async def async_step_user(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Server address typed in by hand."""
        errors: dict[str, str] = {}
        if user_input is not None:
            self._host = user_input[CONF_HOST].strip()
            self._port = user_input[CONF_PORT]
            self._ssl = user_input.get(CONF_USE_SSL, False)
            try:
                self._info = await fetch_server_info(
                    self._client_session(), self._base_url
                )
            except MujinMusicConnectionError:
                errors["base"] = "cannot_connect"
            else:
                await self.async_set_unique_id(self._info.instance_id)
                self._abort_if_unique_id_configured(
                    updates={CONF_HOST: self._host, CONF_PORT: self._port}
                )
                return await self.async_step_auth()
        return self.async_show_form(
            step_id="user",
            data_schema=vol.Schema(
                {
                    vol.Required(CONF_HOST, default=self._host or ""): str,
                    vol.Required(CONF_PORT, default=self._port): vol.All(
                        vol.Coerce(int), vol.Range(min=1, max=65535)
                    ),
                    vol.Optional(CONF_USE_SSL, default=self._ssl): bool,
                }
            ),
            errors=errors,
        )

    async def async_step_zeroconf(
        self, discovery_info: ZeroconfServiceInfo
    ) -> ConfigFlowResult:
        """A server announced itself on the network."""
        properties = discovery_info.properties
        if properties.get("product") not in (None, PRODUCT_ID):
            return self.async_abort(reason="not_mujin_music")
        instance_id = properties.get("id")
        if not instance_id:
            return self.async_abort(reason="not_mujin_music")
        if discovery_info.ip_address.version == 6:
            return self.async_abort(reason="ipv6_not_supported")
        self._host = str(discovery_info.ip_address)
        self._port = discovery_info.port or DEFAULT_PORT
        await self.async_set_unique_id(instance_id)
        # A server that moved to a new address is followed, not re-added.
        self._abort_if_unique_id_configured(
            updates={CONF_HOST: self._host, CONF_PORT: self._port}
        )
        try:
            self._info = await fetch_server_info(self._client_session(), self._base_url)
        except MujinMusicConnectionError:
            return self.async_abort(reason="cannot_connect")
        self.context["title_placeholders"] = {"name": self._info.name}
        return await self.async_step_zeroconf_confirm()

    async def async_step_zeroconf_confirm(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        assert self._info is not None
        if user_input is not None:
            return await self.async_step_auth()
        return self.async_show_form(
            step_id="zeroconf_confirm",
            description_placeholders={
                "name": self._info.name,
                "host": self._host or "",
                "brand": BRAND,
            },
        )

    async def async_step_auth(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Pair (recommended) or sign in with a password."""
        return self.async_show_menu(step_id="auth", menu_options=["pair", "login"])

    async def async_step_pair(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Show a code; the user approves it in the Mujin Music app."""
        errors: dict[str, str] = {}
        session = self._client_session()
        if user_input is not None and self._pairing is not None:
            try:
                tokens = await self._wait_for_approval(self._pairing)
            except MujinMusicAuthError:
                self._pairing = None
                errors["base"] = "pairing_expired"
            except MujinMusicConnectionError:
                errors["base"] = "cannot_connect"
            else:
                if tokens is not None:
                    return await self._finish(tokens)
                errors["base"] = "pairing_pending"
        if self._pairing is None:
            try:
                self._pairing = await start_pairing(
                    session, self._base_url, PAIRING_DEVICE_NAME
                )
            except MujinMusicConnectionError:
                return self.async_abort(reason="cannot_connect")
            except MujinMusicCommandError:
                return self.async_abort(reason="pairing_unavailable")
        return self.async_show_form(
            step_id="pair",
            description_placeholders={
                "code": f"{self._pairing.code[:3]} {self._pairing.code[3:]}",
                "name": self._info.name if self._info else BRAND,
                "minutes": str(max(1, self._pairing.expires_in // 60)),
            },
            errors=errors,
        )

    async def _wait_for_approval(self, pairing: Pairing) -> Tokens | None:
        loop = asyncio.get_running_loop()
        deadline = loop.time() + PAIRING_WAIT_SECONDS
        while True:
            tokens = await pairing_status(self._client_session(), self._base_url, pairing)
            if tokens is not None or loop.time() >= deadline:
                return tokens
            await asyncio.sleep(PAIRING_POLL_SECONDS)

    async def async_step_login(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        """Username and password (kept only long enough to sign in)."""
        errors: dict[str, str] = {}
        if user_input is not None:
            try:
                tokens = await login(
                    self._client_session(),
                    self._base_url,
                    user_input[CONF_USERNAME],
                    user_input[CONF_PASSWORD],
                )
            except MujinMusicAuthError:
                errors["base"] = "invalid_auth"
            except MujinMusicCommandError:
                errors["base"] = "too_many_attempts"
            except MujinMusicConnectionError:
                errors["base"] = "cannot_connect"
            else:
                return await self._finish(tokens)
        return self.async_show_form(
            step_id="login",
            data_schema=vol.Schema(
                {
                    vol.Required(CONF_USERNAME): str,
                    vol.Required(CONF_PASSWORD): str,
                }
            ),
            errors=errors,
        )

    async def _finish(self, tokens: Tokens) -> ConfigFlowResult:
        assert self._info is not None and self._host is not None
        data = {
            CONF_HOST: self._host,
            CONF_PORT: self._port,
            CONF_USE_SSL: self._ssl,
            CONF_INSTANCE_ID: self._info.instance_id,
            CONF_ACCESS_TOKEN: tokens.access_token,
            CONF_REFRESH_TOKEN: tokens.refresh_token,
        }
        if self.source == SOURCE_REAUTH:
            return self.async_update_reload_and_abort(
                self._get_reauth_entry(), data_updates=data
            )
        return self.async_create_entry(title=self._info.name, data=data)

    async def async_step_reauth(
        self, entry_data: Mapping[str, Any]
    ) -> ConfigFlowResult:
        """The saved session expired or was revoked in the app."""
        self._host = entry_data[CONF_HOST]
        self._port = entry_data[CONF_PORT]
        self._ssl = entry_data.get(CONF_USE_SSL, False)
        return await self.async_step_reauth_confirm()

    async def async_step_reauth_confirm(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        if user_input is None:
            return self.async_show_form(
                step_id="reauth_confirm",
                description_placeholders={"name": self._get_reauth_entry().title},
            )
        try:
            self._info = await fetch_server_info(self._client_session(), self._base_url)
        except MujinMusicConnectionError:
            return self.async_abort(reason="cannot_connect")
        await self.async_set_unique_id(self._info.instance_id)
        self._abort_if_unique_id_mismatch(reason="wrong_server")
        return await self.async_step_auth()
