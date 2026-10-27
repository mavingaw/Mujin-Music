"""Async client for a Mujin Music server.

Token auth: a paired device (or a username/password sign-in) gets an
access token and a single-use refresh token. Every request carries the
access token as a bearer token; a 401 rotates both tokens once and retries.
"""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
import logging
from typing import Any
from urllib.parse import quote

import aiohttp

from .const import API_PREFIX, NATIVE_CLIENT_HEADER

_LOGGER = logging.getLogger(__name__)

REQUEST_TIMEOUT = aiohttp.ClientTimeout(total=15)
IMAGE_TIMEOUT = aiohttp.ClientTimeout(total=20)
MAX_IMAGE_BYTES = 16 * 1024 * 1024


class MujinMusicError(Exception):
    """The server could not be reached or refused a request."""


class MujinMusicConnectionError(MujinMusicError):
    """The server is unreachable."""


class MujinMusicAuthError(MujinMusicError):
    """The tokens were rejected and could not be renewed."""


class MujinMusicCommandError(MujinMusicError):
    """The server refused a command (with its error code)."""

    def __init__(self, code: str, status: int) -> None:
        super().__init__(code)
        self.code = code
        self.status = status


@dataclass(slots=True)
class Tokens:
    """An access token and its refresh token."""

    access_token: str
    refresh_token: str


@dataclass(slots=True)
class ServerInfo:
    """What `/.well-known/parson` says about a server."""

    instance_id: str
    name: str
    version: str
    capabilities: list[str]


@dataclass(slots=True)
class Pairing:
    """A pairing request waiting for approval in the Mujin Music app."""

    pairing_id: str
    secret: str
    code: str
    expires_in: int


def server_url(host: str, port: int, ssl: bool = False) -> str:
    """Base URL for a host and port (IPv6 literals get brackets)."""
    if ":" in host and not host.startswith("["):
        host = f"[{host}]"
    return f"{'https' if ssl else 'http'}://{host}:{port}"


def quote_id(value: str) -> str:
    """Quote a zone id for a URL path (`client:…`, `sonos:…` keep their colon)."""
    return quote(value, safe=":")


async def fetch_server_info(session: aiohttp.ClientSession, base_url: str) -> ServerInfo:
    """Reads the discovery manifest; raises if this is not a Mujin Music server."""
    try:
        async with session.get(
            f"{base_url}/.well-known/parson", timeout=REQUEST_TIMEOUT
        ) as response:
            if response.status != 200:
                raise MujinMusicConnectionError(f"manifest status {response.status}")
            manifest = await response.json(content_type=None)
    except (aiohttp.ClientError, TimeoutError, ValueError) as error:
        raise MujinMusicConnectionError(str(error)) from error
    if not isinstance(manifest, dict) or manifest.get("protocol") != "parson":
        raise MujinMusicConnectionError("not a Mujin Music server")
    return ServerInfo(
        instance_id=str(manifest.get("instanceId", "")),
        name=str(manifest.get("name") or "Mujin Music"),
        version=str(manifest.get("serverVersion", "")),
        capabilities=[str(item) for item in manifest.get("capabilities", [])],
    )


def _tokens_from(payload: Any) -> Tokens:
    if (
        not isinstance(payload, dict)
        or not payload.get("status")
        or not payload.get("access_token")
        or not payload.get("refresh_token")
    ):
        raise MujinMusicAuthError(
            (payload or {}).get("message", "sign-in refused")
            if isinstance(payload, dict)
            else "sign-in refused"
        )
    return Tokens(payload["access_token"], payload["refresh_token"])


async def login(
    session: aiohttp.ClientSession, base_url: str, username: str, password: str
) -> Tokens:
    """Signs in as a native client so the tokens come back in the body."""
    try:
        async with session.post(
            f"{base_url}{API_PREFIX}/auth/login",
            json={"username": username, "password": password},
            headers={NATIVE_CLIENT_HEADER: "native"},
            timeout=REQUEST_TIMEOUT,
        ) as response:
            payload = await response.json(content_type=None)
            if response.status in (401, 403):
                raise MujinMusicAuthError(str(payload.get("message", "invalid_auth")))
            if response.status == 429:
                raise MujinMusicCommandError("too_many_attempts", 429)
            if response.status != 200:
                raise MujinMusicConnectionError(f"login status {response.status}")
    except (aiohttp.ClientError, TimeoutError, ValueError) as error:
        raise MujinMusicConnectionError(str(error)) from error
    return _tokens_from(payload)


async def start_pairing(
    session: aiohttp.ClientSession, base_url: str, device_name: str
) -> Pairing:
    """Asks the server for a six-digit pairing code."""
    try:
        async with session.post(
            f"{base_url}{API_PREFIX}/auth/pairing/start",
            json={"device_name": device_name},
            timeout=REQUEST_TIMEOUT,
        ) as response:
            payload = await response.json(content_type=None)
            if response.status != 200:
                raise MujinMusicCommandError(
                    str(payload.get("message", "pairing_unavailable")), response.status
                )
    except (aiohttp.ClientError, TimeoutError, ValueError) as error:
        raise MujinMusicConnectionError(str(error)) from error
    return Pairing(
        pairing_id=payload["pairingId"],
        secret=payload["secret"],
        code=payload["code"],
        expires_in=int(payload.get("expiresIn", 180)),
    )


async def pairing_status(
    session: aiohttp.ClientSession, base_url: str, pairing: Pairing
) -> Tokens | None:
    """Tokens once the pairing is approved, `None` while it is pending."""
    try:
        async with session.post(
            f"{base_url}{API_PREFIX}/auth/pairing/status",
            json={"pairing_id": pairing.pairing_id, "secret": pairing.secret},
            timeout=REQUEST_TIMEOUT,
        ) as response:
            payload = await response.json(content_type=None)
            if response.status == 202:
                return None
            if response.status == 404:
                raise MujinMusicAuthError("pairing_expired")
            if response.status != 200:
                raise MujinMusicConnectionError(f"pairing status {response.status}")
    except (aiohttp.ClientError, TimeoutError, ValueError) as error:
        raise MujinMusicConnectionError(str(error)) from error
    return _tokens_from(payload)


class MujinMusicClient:
    """One signed-in connection to a server."""

    def __init__(
        self,
        session: aiohttp.ClientSession,
        base_url: str,
        tokens: Tokens,
        on_tokens: Callable[[Tokens], Awaitable[None] | None] | None = None,
    ) -> None:
        self.session = session
        self.base_url = base_url.rstrip("/")
        self.tokens = tokens
        self._on_tokens = on_tokens
        self._refresh_lock = asyncio.Lock()

    @property
    def api_url(self) -> str:
        return f"{self.base_url}{API_PREFIX}"

    def _headers(self) -> dict[str, str]:
        return {"Authorization": f"Bearer {self.tokens.access_token}"}

    async def refresh(self, rejected_access_token: str | None = None) -> None:
        """Rotates the tokens (once, even when several requests got a 401)."""
        async with self._refresh_lock:
            if (
                rejected_access_token is not None
                and rejected_access_token != self.tokens.access_token
            ):
                return  # Someone else already rotated them.
            try:
                async with self.session.post(
                    f"{self.api_url}/auth/refresh",
                    headers={
                        "Authorization": f"Bearer {self.tokens.refresh_token}",
                        NATIVE_CLIENT_HEADER: "native",
                    },
                    timeout=REQUEST_TIMEOUT,
                ) as response:
                    payload = await response.json(content_type=None)
                    if response.status in (401, 403):
                        raise MujinMusicAuthError("refresh_rejected")
                    if response.status != 200:
                        raise MujinMusicConnectionError(
                            f"refresh status {response.status}"
                        )
            except (aiohttp.ClientError, TimeoutError, ValueError) as error:
                raise MujinMusicConnectionError(str(error)) from error
            self.tokens = _tokens_from(payload)
            if self._on_tokens is not None:
                result = self._on_tokens(self.tokens)
                if asyncio.iscoroutine(result):
                    await result

    async def request(
        self,
        method: str,
        path: str,
        *,
        json: Any = None,
        params: dict[str, str] | None = None,
        expect_json: bool = True,
    ) -> Any:
        """A JSON API call with one token rotation on 401."""
        for attempt in range(2):
            access_token = self.tokens.access_token
            try:
                async with self.session.request(
                    method,
                    f"{self.api_url}{path}",
                    json=json,
                    params=params,
                    headers=self._headers(),
                    timeout=REQUEST_TIMEOUT,
                ) as response:
                    if response.status == 401 and attempt == 0:
                        await self.refresh(access_token)
                        continue
                    if response.status == 401:
                        raise MujinMusicAuthError("unauthorized")
                    if response.status >= 400:
                        code = "request_failed"
                        try:
                            payload = await response.json(content_type=None)
                            if isinstance(payload, dict):
                                code = str(payload.get("error") or code)
                        except ValueError:
                            pass
                        raise MujinMusicCommandError(code, response.status)
                    if not expect_json or response.status == 204:
                        return None
                    return await response.json(content_type=None)
            except (aiohttp.ClientError, TimeoutError) as error:
                raise MujinMusicConnectionError(str(error)) from error
        raise MujinMusicAuthError("unauthorized")

    # Zones -----------------------------------------------------------------

    async def zones(self) -> dict[str, Any]:
        return await self.request("GET", "/zones")

    async def command(self, zone_id: str, command: str, **arguments: Any) -> None:
        await self.request(
            "POST",
            f"/zones/{quote_id(zone_id)}/commands",
            json={"command": command, **arguments},
        )

    async def transfer(self, from_zone: str, to_zone: str) -> None:
        await self.request(
            "POST", "/zones/transfer", json={"from": from_zone, "to": to_zone}
        )

    async def group(self, zone_ids: list[str]) -> dict[str, Any]:
        return await self.request("POST", "/zones/groups", json={"zone_ids": zone_ids})

    async def ungroup(self, group_id: str, member: str | None = None) -> None:
        await self.request(
            "DELETE",
            f"/zones/groups/{quote_id(group_id)}",
            params={"member": member} if member else None,
            expect_json=False,
        )

    async def sleep_timer(self, zone_id: str, minutes: int | None) -> None:
        await self.request(
            "POST",
            f"/zones/{quote_id(zone_id)}/sleep",
            json={"minutes": minutes},
            expect_json=False,
        )

    async def pause_all(self) -> int:
        result = await self.request("POST", "/zones/pause-all")
        return int((result or {}).get("paused", 0))

    # Library ---------------------------------------------------------------

    async def browse(self, item: str | None = None) -> dict[str, Any]:
        return await self.request("GET", "/browse", params={"item": item or "root"})

    async def search(self, query: str) -> dict[str, Any]:
        return await self.request("GET", "/browse", params={"query": query})

    async def play(
        self,
        zone_id: str,
        *,
        item: str | None = None,
        query: str | None = None,
        kind: str | None = None,
        action: str = "play",
    ) -> dict[str, Any]:
        body: dict[str, Any] = {"action": action}
        if item:
            body["item"] = item
        if query:
            body["query"] = query
        if kind:
            body["kind"] = kind
        return await self.request("POST", f"/zones/{quote_id(zone_id)}/play", json=body)

    async def radio(
        self,
        zone_id: str,
        *,
        item: str | None = None,
        query: str | None = None,
        kind: str | None = None,
    ) -> dict[str, Any]:
        body: dict[str, Any] = {"action": "radio"}
        if item:
            body["item"] = item
        if query:
            body["query"] = query
        if kind:
            body["kind"] = kind
        return await self.request(
            "POST", f"/zones/{quote_id(zone_id)}/radio", json=body
        )

    async def rate(self, zone_id: str, rating: str) -> dict[str, Any]:
        return await self.request(
            "POST", f"/zones/{quote_id(zone_id)}/rating", json={"rating": rating}
        )

    async def image(self, path: str) -> tuple[bytes, str] | None:
        """Original artwork bytes (lossless covers stay lossless)."""
        for attempt in range(2):
            access_token = self.tokens.access_token
            try:
                async with self.session.get(
                    f"{self.base_url}/media/images/{quote(path, safe='')}",
                    params={"raw": "true"},
                    headers=self._headers(),
                    timeout=IMAGE_TIMEOUT,
                ) as response:
                    if response.status == 401 and attempt == 0:
                        await self.refresh(access_token)
                        continue
                    if response.status != 200:
                        return None
                    if (response.content_length or 0) > MAX_IMAGE_BYTES:
                        return None
                    body = await response.content.read(MAX_IMAGE_BYTES + 1)
                    if len(body) > MAX_IMAGE_BYTES:
                        return None
                    return body, response.content_type or "image/jpeg"
            except (aiohttp.ClientError, TimeoutError) as error:
                _LOGGER.debug("Artwork %s unavailable: %s", path, error)
                return None
        return None

    async def connect_events(self) -> aiohttp.ClientWebSocketResponse:
        """Opens the zone event stream (rotating tokens once on 401)."""
        url = f"{self.api_url}/zones/events".replace("http", "ws", 1)
        for attempt in range(2):
            access_token = self.tokens.access_token
            try:
                return await self.session.ws_connect(
                    url, headers=self._headers(), heartbeat=30, timeout=aiohttp.ClientWSTimeout(ws_close=10)
                )
            except aiohttp.WSServerHandshakeError as error:
                if error.status == 401 and attempt == 0:
                    await self.refresh(access_token)
                    continue
                if error.status == 401:
                    raise MujinMusicAuthError("unauthorized") from error
                raise MujinMusicConnectionError(str(error)) from error
            except (aiohttp.ClientError, TimeoutError) as error:
                raise MujinMusicConnectionError(str(error)) from error
        raise MujinMusicAuthError("unauthorized")
