"""Zone state for one server, pushed over the server's event stream."""

from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
from datetime import datetime, timedelta
import logging
from typing import Any

import aiohttp

from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant, callback
from homeassistant.exceptions import ConfigEntryAuthFailed
from homeassistant.helpers.update_coordinator import DataUpdateCoordinator, UpdateFailed
from homeassistant.util import dt as dt_util

from .api import (
    MujinMusicAuthError,
    MujinMusicClient,
    MujinMusicError,
)
from .const import (
    DOMAIN,
    EVENT_DEBOUNCE_SECONDS,
    FALLBACK_POLL_SECONDS,
    RECONNECT_MAX_SECONDS,
    RECONNECT_MIN_SECONDS,
)

_LOGGER = logging.getLogger(__name__)


@dataclass(slots=True)
class ZoneSnapshot:
    """Every zone the account can see, as of `fetched_at`."""

    zones: dict[str, dict[str, Any]] = field(default_factory=dict)
    fetched_at: datetime = field(default_factory=dt_util.utcnow)

    def zone(self, zone_id: str) -> dict[str, Any] | None:
        return self.zones.get(zone_id)

    def group_containing(self, zone_id: str) -> dict[str, Any] | None:
        """The group zone that lists `zone_id` as a member (not itself)."""
        for zone in self.zones.values():
            group = zone.get("group") or {}
            if zone["id"] != zone_id and any(
                member.get("id") == zone_id for member in group.get("members", [])
            ):
                return zone
        return None


type MujinMusicConfigEntry = ConfigEntry[MujinMusicCoordinator]


class MujinMusicCoordinator(DataUpdateCoordinator[ZoneSnapshot]):
    """Fetches zones on every server event; polls only as a fallback."""

    config_entry: MujinMusicConfigEntry

    def __init__(
        self,
        hass: HomeAssistant,
        entry: MujinMusicConfigEntry,
        client: MujinMusicClient,
        server_name: str,
    ) -> None:
        super().__init__(
            hass,
            _LOGGER,
            config_entry=entry,
            name=f"{DOMAIN} {server_name}",
            update_interval=timedelta(seconds=FALLBACK_POLL_SECONDS),
        )
        self.client = client
        self.server_name = server_name
        self.connected = False
        self._listener: asyncio.Task[None] | None = None
        self._debounce: asyncio.TimerHandle | None = None
        self._stopping = False
        # Zone ids ever seen, so a zone that disappears (a phone that went
        # to sleep) keeps its entity and shows as unavailable.
        self.known_zone_ids: set[str] = set()

    async def _async_update_data(self) -> ZoneSnapshot:
        try:
            payload = await self.client.zones()
        except MujinMusicAuthError as error:
            raise ConfigEntryAuthFailed(str(error)) from error
        except MujinMusicError as error:
            raise UpdateFailed(f"Mujin Music is unreachable: {error}") from error
        zones = {
            zone["id"]: zone
            for zone in payload.get("zones", [])
            if isinstance(zone, dict) and "id" in zone
        }
        self.known_zone_ids.update(zones)
        return ZoneSnapshot(zones=zones, fetched_at=dt_util.utcnow())

    @callback
    def start_events(self) -> None:
        """Starts the push listener (reconnects with backoff until stopped)."""
        if self._listener is None:
            self._listener = self.config_entry.async_create_background_task(
                self.hass, self._listen(), f"{DOMAIN}_events"
            )

    async def stop_events(self) -> None:
        self._stopping = True
        if self._debounce is not None:
            self._debounce.cancel()
            self._debounce = None
        if self._listener is not None:
            self._listener.cancel()
            try:
                await self._listener
            except asyncio.CancelledError:
                pass
            self._listener = None

    @callback
    def _schedule_refresh(self) -> None:
        if self._debounce is not None:
            self._debounce.cancel()
        self._debounce = self.hass.loop.call_later(
            EVENT_DEBOUNCE_SECONDS,
            lambda: self.hass.async_create_task(
                self.async_refresh(), eager_start=True
            ),
        )

    async def _listen(self) -> None:
        delay = RECONNECT_MIN_SECONDS
        while not self._stopping:
            try:
                socket = await self.client.connect_events()
            except MujinMusicAuthError:
                _LOGGER.warning("Mujin Music rejected the saved sign-in")
                self.config_entry.async_start_reauth(self.hass)
                return
            except MujinMusicError as error:
                _LOGGER.debug("Event stream unavailable (%s); retrying", error)
                await asyncio.sleep(delay)
                delay = min(delay * 2, RECONNECT_MAX_SECONDS)
                continue
            delay = RECONNECT_MIN_SECONDS
            self.connected = True
            try:
                async for message in socket:
                    if message.type == aiohttp.WSMsgType.TEXT:
                        try:
                            event = message.json()
                        except ValueError:
                            continue
                        # "ready" means we may have missed changes while
                        # disconnected; "zones_changed" is the push itself.
                        if event.get("type") in ("ready", "zones_changed"):
                            self._schedule_refresh()
                    elif message.type in (
                        aiohttp.WSMsgType.CLOSED,
                        aiohttp.WSMsgType.ERROR,
                    ):
                        break
            finally:
                self.connected = False
                if not socket.closed:
                    await socket.close()
            if not self._stopping:
                await asyncio.sleep(delay)
