"""Every Mujin Music zone as a Home Assistant media player."""

from __future__ import annotations

from collections.abc import Callable, Coroutine
from datetime import datetime
import functools
import logging
from typing import Any, Concatenate
from urllib.parse import quote

import voluptuous as vol

from homeassistant.components.media_player import (
    ATTR_MEDIA_ENQUEUE,
    BrowseError,
    BrowseMedia,
    MediaClass,
    MediaPlayerEnqueue,
    MediaPlayerEntity,
    MediaPlayerEntityFeature,
    MediaPlayerState,
    MediaType,
    RepeatMode,
    SearchMedia,
    SearchMediaQuery,
)
from homeassistant.core import HomeAssistant, ServiceResponse, SupportsResponse, callback
from homeassistant.exceptions import HomeAssistantError, ServiceValidationError
from homeassistant.helpers import (
    config_validation as cv,
    device_registry as dr,
    entity_platform,
    entity_registry as er,
)
from homeassistant.helpers.device_registry import DeviceInfo
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity

from .api import MujinMusicCommandError, MujinMusicError
from .const import (
    ATTR_DESTINATION,
    ATTR_ITEM,
    ATTR_KIND,
    ATTR_MINUTES,
    ATTR_QUERY,
    BRAND,
    CONF_INSTANCE_ID,
    DOMAIN,
    MANUFACTURER,
    SEARCH_KINDS,
    SERVICE_SLEEP_TIMER,
    SERVICE_START_RADIO,
    SERVICE_THUMBS_DOWN,
    SERVICE_THUMBS_UP,
    SERVICE_TRANSFER,
)
from .coordinator import MujinMusicConfigEntry, MujinMusicCoordinator

_LOGGER = logging.getLogger(__name__)

PARALLEL_UPDATES = 0

# Zones that only exist while they play or stay grouped: their entities are
# removed when they go away instead of lingering as unavailable.
TRANSIENT_PREFIXES = ("linked:", "bridge-group:", "cast:")

# Browse ids the server understands; anything else in play_media is a search.
ITEM_PREFIXES = (
    "album:",
    "artist:",
    "playlist:",
    "genre:",
    "song:",
    "radio:",
)
ITEM_IDS = ("liked",)

KIND_MEDIA_CLASS: dict[str, MediaClass] = {
    "library": MediaClass.DIRECTORY,
    "directory": MediaClass.DIRECTORY,
    "album": MediaClass.ALBUM,
    "artist": MediaClass.ARTIST,
    "playlist": MediaClass.PLAYLIST,
    "genre": MediaClass.GENRE,
    "song": MediaClass.TRACK,
    "radio": MediaClass.CHANNEL,
}
KIND_MEDIA_TYPE: dict[str, str] = {
    "library": "library",
    "directory": "library",
    "album": MediaType.ALBUM,
    "artist": MediaType.ARTIST,
    "playlist": MediaType.PLAYLIST,
    "genre": MediaType.GENRE,
    "song": MediaType.TRACK,
    "radio": MediaType.CHANNEL,
}
# play_media(media_content_type=…) with free text: which kind to search.
SEARCH_KIND_BY_TYPE: dict[str, str] = {
    MediaType.ARTIST: "artist",
    MediaType.ALBUM: "album",
    MediaType.TRACK: "song",
    "song": "song",
    MediaType.PLAYLIST: "playlist",
    MediaType.GENRE: "genre",
}

STATE_BY_STATUS = {
    "playing": MediaPlayerState.PLAYING,
    "paused": MediaPlayerState.PAUSED,
    "buffering": MediaPlayerState.BUFFERING,
    "stopped": MediaPlayerState.IDLE,
    "idle": MediaPlayerState.IDLE,
}
REPEAT_TO_HA = {"off": RepeatMode.OFF, "all": RepeatMode.ALL, "one": RepeatMode.ONE}
REPEAT_FROM_HA = {value: key for key, value in REPEAT_TO_HA.items()}
ENQUEUE_ACTION = {
    None: "play",
    MediaPlayerEnqueue.PLAY: "play",
    MediaPlayerEnqueue.REPLACE: "play",
    MediaPlayerEnqueue.NEXT: "next",
    MediaPlayerEnqueue.ADD: "add",
}
KIND_LABEL = {
    "web": "Web player",
    "desktop": "Desktop app",
    "mobile": "Phone",
    "chromecast": "Chromecast",
    "sonos": "Sonos",
    "upnp": "UPnP renderer",
    "device": "Audio device",
    "bridge": "Mujin Music Bridge",
    "airplay": "AirPlay",
    "linked": "Linked zones",
}


def _is_transient(zone_id: str) -> bool:
    return zone_id.startswith(TRANSIENT_PREFIXES)


def _is_hires(quality: str | None) -> bool:
    """`FLAC 24/96` and friends: more than CD resolution."""
    if not quality or "/" not in quality:
        return False
    depth, _, rate = quality.rsplit(" ", 1)[-1].partition("/")
    try:
        return int(depth) > 16 or float(rate) > 48
    except ValueError:
        return False


async def async_setup_entry(
    hass: HomeAssistant,
    entry: MujinMusicConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    """Adds a player per zone now, and for every zone that appears later."""
    coordinator = entry.runtime_data
    instance_id = entry.data[CONF_INSTANCE_ID]
    device_registry = dr.async_get(hass)
    device_registry.async_get_or_create(
        config_entry_id=entry.entry_id,
        identifiers={(DOMAIN, instance_id)},
        name=entry.title,
        manufacturer=MANUFACTURER,
        model="Music server",
    )
    entities: dict[str, MujinMusicZone] = {}

    # Players for zones seen in earlier sessions come back (unavailable
    # until their zone returns), so automations keep their targets.
    entity_registry = er.async_get(hass)
    prefix = f"{instance_id}-"
    for registered in er.async_entries_for_config_entry(entity_registry, entry.entry_id):
        if registered.domain != "media_player" or not registered.unique_id.startswith(prefix):
            continue
        zone_id = registered.unique_id.removeprefix(prefix)
        if _is_transient(zone_id) and coordinator.data.zone(zone_id) is None:
            entity_registry.async_remove(registered.entity_id)
            continue
        coordinator.known_zone_ids.add(zone_id)

    @callback
    def sync_entities() -> None:
        snapshot = coordinator.data
        new: list[MujinMusicZone] = []
        present = set(snapshot.zones)
        absorbed = {
            member["id"]
            for zone in snapshot.zones.values()
            for member in (zone.get("group") or {}).get("members", [])
        }
        for zone_id in sorted(coordinator.known_zone_ids | present):
            if zone_id in entities:
                continue
            if zone_id not in present and zone_id not in absorbed and _is_transient(zone_id):
                continue
            entities[zone_id] = MujinMusicZone(coordinator, instance_id, zone_id)
            new.append(entities[zone_id])
        if new:
            async_add_entities(new)
        # Linked groups, bridge groups and cast sessions that ended.
        for zone_id in [
            zone_id
            for zone_id in entities
            if _is_transient(zone_id) and zone_id not in present and zone_id not in absorbed
        ]:
            entity = entities.pop(zone_id)
            coordinator.known_zone_ids.discard(zone_id)
            if entity.registry_entry is not None:
                er.async_get(hass).async_remove(entity.entity_id)
            device = device_registry.async_get_device(
                identifiers={(DOMAIN, f"{instance_id}-{zone_id}")}
            )
            if device is not None:
                device_registry.async_update_device(
                    device.id, remove_config_entry_id=entry.entry_id
                )

    sync_entities()
    entry.async_on_unload(coordinator.async_add_listener(sync_entities))

    platform = entity_platform.async_get_current_platform()
    platform.async_register_entity_service(
        SERVICE_START_RADIO,
        {
            vol.Optional(ATTR_ITEM): cv.string,
            vol.Optional(ATTR_QUERY): cv.string,
            vol.Optional(ATTR_KIND): vol.In(SEARCH_KINDS),
        },
        "async_start_radio",
        supports_response=SupportsResponse.OPTIONAL,
    )
    platform.async_register_entity_service(SERVICE_THUMBS_UP, None, "async_thumbs_up")
    platform.async_register_entity_service(SERVICE_THUMBS_DOWN, None, "async_thumbs_down")
    platform.async_register_entity_service(
        SERVICE_TRANSFER,
        {vol.Required(ATTR_DESTINATION): cv.entity_id},
        "async_transfer",
    )
    platform.async_register_entity_service(
        SERVICE_SLEEP_TIMER,
        {vol.Required(ATTR_MINUTES): vol.All(vol.Coerce(int), vol.Range(min=0, max=720))},
        "async_sleep_timer",
    )


def _command[**P, R](
    func: Callable[Concatenate[MujinMusicZone, P], Coroutine[Any, Any, R]],
) -> Callable[Concatenate[MujinMusicZone, P], Coroutine[Any, Any, R]]:
    """Turns server refusals into readable Home Assistant errors and
    refreshes right away so the UI never waits for the echo."""

    @functools.wraps(func)
    async def wrapper(self: MujinMusicZone, *args: P.args, **kwargs: P.kwargs) -> R:
        try:
            result = await func(self, *args, **kwargs)
        except MujinMusicCommandError as error:
            raise HomeAssistantError(
                translation_domain=DOMAIN,
                translation_key="command_refused",
                translation_placeholders={"error": error.code.replace("_", " ")},
            ) from error
        except MujinMusicError as error:
            raise HomeAssistantError(
                translation_domain=DOMAIN,
                translation_key="command_failed",
                translation_placeholders={"error": str(error)},
            ) from error
        await self.coordinator.async_refresh()
        return result

    return wrapper


class MujinMusicZone(CoordinatorEntity[MujinMusicCoordinator], MediaPlayerEntity):
    """One zone: an app, a Sonos room or group, a renderer, a linked group."""

    _attr_has_entity_name = True
    _attr_name = None
    _attr_media_image_remotely_accessible = False

    def __init__(
        self, coordinator: MujinMusicCoordinator, instance_id: str, zone_id: str
    ) -> None:
        super().__init__(coordinator)
        self.zone_id = zone_id
        self._instance_id = instance_id
        self._attr_unique_id = f"{instance_id}-{zone_id}"
        zone, _ = self._resolve()
        name = (zone or {}).get("name") or self._member_name() or zone_id
        kind = (zone or {}).get("kind", "")
        self._attr_device_info = DeviceInfo(
            identifiers={(DOMAIN, self._attr_unique_id)},
            name=name,
            manufacturer="Sonos" if kind == "sonos" else MANUFACTURER,
            model=(zone or {}).get("device") or KIND_LABEL.get(kind, "Zone"),
            via_device=(DOMAIN, instance_id),
        )

    # State -----------------------------------------------------------------

    def _member_name(self) -> str | None:
        group = self.coordinator.data.group_containing(self.zone_id)
        for member in (group or {}).get("group", {}).get("members", []):
            if member.get("id") == self.zone_id:
                return member.get("name")
        return None

    def _resolve(self) -> tuple[dict[str, Any] | None, dict[str, Any] | None]:
        """(zone to read and control, this zone's member entry when it is a
        room inside another zone's group, e.g. a grouped Sonos room)."""
        snapshot = self.coordinator.data
        zone = snapshot.zone(self.zone_id)
        if zone is not None:
            return zone, None
        group = snapshot.group_containing(self.zone_id)
        if group is None:
            return None, None
        member = next(
            member
            for member in group["group"]["members"]
            if member.get("id") == self.zone_id
        )
        return group, member

    @property
    def _zone(self) -> dict[str, Any]:
        zone, _ = self._resolve()
        if zone is None:
            raise HomeAssistantError(
                translation_domain=DOMAIN,
                translation_key="zone_unavailable",
            )
        return zone

    @property
    def _target(self) -> str:
        """Zone id that transport commands go to."""
        return self._zone["id"]

    @property
    def available(self) -> bool:
        zone, _ = self._resolve()
        return (
            super().available
            and zone is not None
            and zone.get("status") != "unavailable"
        )

    @property
    def state(self) -> MediaPlayerState | None:
        zone, _ = self._resolve()
        if zone is None:
            return None
        return STATE_BY_STATUS.get(zone.get("status", ""), MediaPlayerState.IDLE)

    @property
    def supported_features(self) -> MediaPlayerEntityFeature:
        zone, member = self._resolve()
        if zone is None:
            return MediaPlayerEntityFeature(0)
        capabilities = zone.get("capabilities", {})
        features = MediaPlayerEntityFeature(0)
        if capabilities.get("transport"):
            features |= (
                MediaPlayerEntityFeature.PLAY
                | MediaPlayerEntityFeature.PAUSE
                | MediaPlayerEntityFeature.STOP
                | MediaPlayerEntityFeature.NEXT_TRACK
                | MediaPlayerEntityFeature.PREVIOUS_TRACK
                | MediaPlayerEntityFeature.PLAY_MEDIA
                | MediaPlayerEntityFeature.BROWSE_MEDIA
                | MediaPlayerEntityFeature.MEDIA_ENQUEUE
                | MediaPlayerEntityFeature.SEARCH_MEDIA
            )
        if capabilities.get("seek"):
            features |= MediaPlayerEntityFeature.SEEK
        volume = member.get("volume") if member else zone.get("volume")
        if capabilities.get("volume") and volume:
            features |= MediaPlayerEntityFeature.VOLUME_SET | MediaPlayerEntityFeature.VOLUME_STEP
        if capabilities.get("mute") and volume:
            features |= MediaPlayerEntityFeature.VOLUME_MUTE
        if capabilities.get("shuffle"):
            features |= MediaPlayerEntityFeature.SHUFFLE_SET
        if capabilities.get("repeat"):
            features |= MediaPlayerEntityFeature.REPEAT_SET
        if capabilities.get("group") or zone.get("group") or zone.get("grouped_into"):
            features |= MediaPlayerEntityFeature.GROUPING
        if capabilities.get("queue_edit"):
            features |= MediaPlayerEntityFeature.CLEAR_PLAYLIST
        if capabilities.get("transfer_target") and self._transfer_sources():
            features |= MediaPlayerEntityFeature.SELECT_SOURCE
        return features

    @property
    def _now_playing(self) -> dict[str, Any]:
        zone, _ = self._resolve()
        return (zone or {}).get("now_playing") or {}

    @property
    def media_content_id(self) -> str | None:
        song_id = self._now_playing.get("song_id")
        return f"song:{song_id}" if song_id else None

    @property
    def media_content_type(self) -> MediaType | None:
        return MediaType.MUSIC if self._now_playing else None

    @property
    def media_title(self) -> str | None:
        return self._now_playing.get("title") or None

    @property
    def media_artist(self) -> str | None:
        return self._now_playing.get("artist") or None

    @property
    def media_album_artist(self) -> str | None:
        return self.media_artist

    @property
    def media_album_name(self) -> str | None:
        return self._now_playing.get("album") or None

    @property
    def media_duration(self) -> int | None:
        zone, _ = self._resolve()
        duration = (zone or {}).get("duration_ms") or self._now_playing.get("duration_ms")
        return round(duration / 1000) if duration else None

    @property
    def media_position(self) -> int | None:
        zone, _ = self._resolve()
        if not zone or not self._now_playing:
            return None
        return round(zone.get("position_ms", 0) / 1000)

    @property
    def media_position_updated_at(self) -> datetime | None:
        return self.coordinator.data.fetched_at if self._now_playing else None

    @property
    def media_image_url(self) -> str | None:
        cover = self._now_playing.get("cover_url")
        if not cover:
            return None
        return f"{self.coordinator.client.base_url}/media/images/{quote(cover, safe='')}?raw=true"

    async def async_get_media_image(self) -> tuple[bytes | None, str | None]:
        cover = self._now_playing.get("cover_url")
        if not cover:
            return None, None
        image = await self.coordinator.client.image(cover)
        return image if image else (None, None)

    @property
    def _volume(self) -> dict[str, Any] | None:
        zone, member = self._resolve()
        if member is not None:
            return member.get("volume")
        return (zone or {}).get("volume")

    @property
    def volume_level(self) -> float | None:
        volume = self._volume
        return volume["level"] / 100 if volume else None

    @property
    def is_volume_muted(self) -> bool | None:
        volume = self._volume
        return volume.get("muted") if volume else None

    @property
    def volume_step(self) -> float:
        volume = self._volume
        return (volume or {}).get("step", 2) / 100

    @property
    def shuffle(self) -> bool | None:
        zone, _ = self._resolve()
        return (zone or {}).get("shuffle")

    @property
    def repeat(self) -> RepeatMode | None:
        zone, _ = self._resolve()
        return REPEAT_TO_HA.get((zone or {}).get("repeat", "off"))

    @property
    def source(self) -> str | None:
        zone, _ = self._resolve()
        source = (zone or {}).get("source")
        if source == "parson":
            return BRAND
        if source == "external":
            return "External input"
        return None

    def _transfer_sources(self) -> dict[str, str]:
        """Other zones with something to hand over: name -> zone id."""
        own = self._zone_id_for_commands()
        sources: dict[str, str] = {}
        for zone in self.coordinator.data.zones.values():
            if zone["id"] == own or zone.get("kind") == "linked":
                continue
            if zone.get("source") == "parson" and zone.get("queue", {}).get("song_ids"):
                sources[f"Play from {zone['name']}"] = zone["id"]
        return sources

    @property
    def source_list(self) -> list[str] | None:
        sources = list(self._transfer_sources())
        return sources or None

    def _zone_id_for_commands(self) -> str:
        zone, member = self._resolve()
        if zone is None:
            return self.zone_id
        return zone["id"]

    def _entity_id_for(self, zone_id: str) -> str | None:
        return er.async_get(self.hass).async_get_entity_id(
            "media_player", DOMAIN, f"{self._instance_id}-{zone_id}"
        )

    def _zone_id_for_entity(self, entity_id: str) -> str:
        entry = er.async_get(self.hass).async_get(entity_id)
        prefix = f"{self._instance_id}-"
        if (
            entry is None
            or entry.platform != DOMAIN
            or entry.unique_id is None
            or not entry.unique_id.startswith(prefix)
        ):
            raise ServiceValidationError(
                translation_domain=DOMAIN,
                translation_key="not_a_zone",
                translation_placeholders={"entity_id": entity_id},
            )
        return entry.unique_id.removeprefix(prefix)

    def _group_zone(self) -> dict[str, Any] | None:
        """The zone that holds this one's group (itself when it leads)."""
        zone, member = self._resolve()
        if zone is None:
            return None
        if member is not None or zone.get("group"):
            return zone
        if grouped_into := zone.get("grouped_into"):
            return self.coordinator.data.zone(grouped_into)
        return None

    @property
    def group_members(self) -> list[str] | None:
        group_zone = self._group_zone()
        if group_zone is None:
            return None
        members = [
            entity_id
            for member in (group_zone.get("group") or {}).get("members", [])
            if (entity_id := self._entity_id_for(member["id"])) is not None
        ]
        return members or None

    @property
    def extra_state_attributes(self) -> dict[str, Any]:
        zone, member = self._resolve()
        if zone is None:
            return {"zone_id": self.zone_id}
        attributes: dict[str, Any] = {
            "zone_id": self.zone_id,
            "zone_kind": KIND_LABEL.get(zone.get("kind", ""), zone.get("kind")),
            "device": zone.get("device") or None,
        }
        if member is not None:
            attributes["playing_in_group"] = zone.get("name")
        queue = zone.get("queue") or {}
        if queue.get("song_ids"):
            attributes["queue_size"] = len(queue["song_ids"])
            attributes["queue_position"] = queue.get("current_position", 0) + 1
        stream = (zone.get("output") or {}).get("stream")
        if stream:
            # Signal path: the same quality light the Mujin Music apps show.
            attributes["quality"] = stream.get("quality")
            attributes["bit_perfect"] = stream.get("bit_perfect")
            attributes["source_format"] = stream.get("source")
            attributes["output_format"] = stream.get("output")
            attributes["signal_path"] = stream.get("steps")
        if group := zone.get("group"):
            attributes["group_synced"] = group.get("synced")
            attributes["group_note"] = group.get("sync_note")
        if timer := zone.get("sleep_timer"):
            attributes["sleep_timer_ends_at"] = datetime.fromtimestamp(
                timer["ends_at_ms"] / 1000
            ).astimezone().isoformat()
        volume = self._volume
        if volume and volume.get("comfort") is not None:
            attributes["volume_comfort_limit"] = volume["comfort"] / 100
        return {key: value for key, value in attributes.items() if value is not None}

    # Transport -------------------------------------------------------------

    async def _send(self, command: str, **arguments: Any) -> None:
        await self.coordinator.client.command(self._target, command, **arguments)

    @_command
    async def async_media_play(self) -> None:
        await self._send("play")

    @_command
    async def async_media_pause(self) -> None:
        await self._send("pause")

    @_command
    async def async_media_play_pause(self) -> None:
        await self._send("play_pause")

    @_command
    async def async_media_stop(self) -> None:
        await self._send("stop")

    @_command
    async def async_media_next_track(self) -> None:
        await self._send("next")

    @_command
    async def async_media_previous_track(self) -> None:
        await self._send("previous")

    @_command
    async def async_media_seek(self, position: float) -> None:
        await self._send("seek", position_ms=max(0, round(position * 1000)))

    def _volume_member(self) -> dict[str, Any]:
        _, member = self._resolve()
        return {"member_id": member["id"]} if member is not None else {}

    @_command
    async def async_set_volume_level(self, volume: float) -> None:
        await self._send(
            "set_volume",
            volume=max(0, min(100, round(volume * 100))),
            **self._volume_member(),
        )

    @_command
    async def async_volume_up(self) -> None:
        step = (self._volume or {}).get("step", 2)
        await self._send("adjust_volume", delta=step, **self._volume_member())

    @_command
    async def async_volume_down(self) -> None:
        step = (self._volume or {}).get("step", 2)
        await self._send("adjust_volume", delta=-step, **self._volume_member())

    @_command
    async def async_mute_volume(self, mute: bool) -> None:
        await self._send("set_mute", muted=mute, **self._volume_member())

    @_command
    async def async_set_shuffle(self, shuffle: bool) -> None:
        await self._send("set_shuffle", enabled=shuffle)

    @_command
    async def async_set_repeat(self, repeat: RepeatMode) -> None:
        await self._send("set_repeat", mode=REPEAT_FROM_HA[RepeatMode(repeat)])

    @_command
    async def async_clear_playlist(self) -> None:
        await self._send("clear_upcoming")

    @_command
    async def async_select_source(self, source: str) -> None:
        sources = self._transfer_sources()
        if source not in sources:
            raise ServiceValidationError(
                translation_domain=DOMAIN,
                translation_key="unknown_source",
                translation_placeholders={"source": source},
            )
        await self.coordinator.client.transfer(sources[source], self._target)

    # Grouping and transfer ---------------------------------------------------

    @_command
    async def async_join_players(self, group_members: list[str]) -> None:
        own = self._zone_id_for_commands()
        others = [self._zone_id_for_entity(entity_id) for entity_id in group_members]
        group_zone = self._group_zone()
        zone_ids = [own]
        if group_zone is not None:
            zone_ids = [member["id"] for member in group_zone["group"]["members"]]
            if group_zone["id"].startswith("linked:"):
                # Linked groups do not nest: regroup with everyone.
                await self.coordinator.client.ungroup(group_zone["id"])
            elif own not in zone_ids:
                zone_ids.insert(0, own)
        for zone_id in others:
            if zone_id not in zone_ids:
                zone_ids.append(zone_id)
        if len(zone_ids) < 2:
            return
        await self.coordinator.client.group(zone_ids)

    @_command
    async def async_unjoin_player(self) -> None:
        zone, member = self._resolve()
        if zone is None:
            return
        if zone.get("kind") == "linked" and member is None:
            await self.coordinator.client.ungroup(zone["id"])
        elif member is not None:
            await self.coordinator.client.ungroup(zone["id"], member["id"])
        elif grouped_into := zone.get("grouped_into"):
            await self.coordinator.client.ungroup(grouped_into, zone["id"])
        elif zone.get("group") and len(zone["group"].get("members", [])) > 1:
            await self.coordinator.client.ungroup(zone["id"], zone["id"])

    @_command
    async def async_transfer(self, destination: str) -> None:
        """Moves this zone's queue and position to another zone."""
        to_zone = self._zone_id_for_entity(destination)
        await self.coordinator.client.transfer(self._target, to_zone)

    @_command
    async def async_sleep_timer(self, minutes: int) -> None:
        await self.coordinator.client.sleep_timer(self._target, minutes or None)

    # Radio and ratings ---------------------------------------------------------

    @_command
    async def async_start_radio(
        self,
        item: str | None = None,
        query: str | None = None,
        kind: str | None = None,
    ) -> ServiceResponse:
        result = await self.coordinator.client.radio(
            self._target, item=item, query=query, kind=kind
        )
        return {"station": result.get("title"), "songs": result.get("songs")}

    @_command
    async def async_thumbs_up(self) -> None:
        await self.coordinator.client.rate(self._target, "up")

    @_command
    async def async_thumbs_down(self) -> None:
        await self.coordinator.client.rate(self._target, "down")

    # Library ---------------------------------------------------------------------

    @_command
    async def async_play_media(
        self, media_type: MediaType | str, media_id: str, **kwargs: Any
    ) -> None:
        if media_id.startswith("media-source://"):
            raise ServiceValidationError(
                translation_domain=DOMAIN, translation_key="media_source_unsupported"
            )
        action = ENQUEUE_ACTION.get(kwargs.get(ATTR_MEDIA_ENQUEUE), "play")
        client = self.coordinator.client
        if media_id.startswith(ITEM_PREFIXES) or media_id in ITEM_IDS:
            await client.play(self._target, item=media_id, action=action)
            return
        if media_type in (MediaType.CHANNEL, "radio"):
            await client.radio(self._target, query=media_id)
            return
        await client.play(
            self._target,
            query=media_id,
            kind=SEARCH_KIND_BY_TYPE.get(str(media_type)),
            action=action,
        )

    def _browse_item(self, item: dict[str, Any], parent_kind: str) -> BrowseMedia:
        kind = item.get("kind", "directory")
        title = item["title"]
        subtitle = item.get("subtitle")
        quality = item.get("quality")
        if kind == "album" and parent_kind != "artist" and subtitle:
            title = f"{title} · {subtitle}"
        if kind == "song" and parent_kind != "album" and subtitle:
            title = f"{title} · {subtitle}"
        if kind == "album" and _is_hires(quality):
            title = f"{title} · {quality}"
        thumbnail = None
        if image := item.get("image"):
            thumbnail = self.get_browse_image_url(
                KIND_MEDIA_TYPE.get(kind, "library"), item["id"], media_image_id=image
            )
        return BrowseMedia(
            media_class=KIND_MEDIA_CLASS.get(kind, MediaClass.DIRECTORY),
            media_content_id=item["id"],
            media_content_type=KIND_MEDIA_TYPE.get(kind, "library"),
            title=title,
            can_play=bool(item.get("playable")),
            can_expand=bool(item.get("expandable")),
            thumbnail=thumbnail,
        )

    async def async_browse_media(
        self,
        media_content_type: MediaType | str | None = None,
        media_content_id: str | None = None,
    ) -> BrowseMedia:
        try:
            response = await self.coordinator.client.browse(media_content_id)
        except MujinMusicCommandError as error:
            raise BrowseError(f"Cannot browse {media_content_id}: {error.code}") from error
        except MujinMusicError as error:
            raise BrowseError(str(error)) from error
        node = response["item"]
        root = self._browse_item(node, "")
        if media_content_id in (None, "", "root"):
            root.title = BRAND if not node.get("title") else f"{BRAND} · {node['title']}"
        children = [
            self._browse_item(child, node.get("kind", "")) for child in response["children"]
        ]
        root.children = children
        classes = {child.media_class for child in children}
        root.children_media_class = (
            classes.pop() if len(classes) == 1 else MediaClass.DIRECTORY
        )
        return root

    async def async_get_browse_image(
        self,
        media_content_type: MediaType | str,
        media_content_id: str,
        media_image_id: str | None = None,
    ) -> tuple[bytes | None, str | None]:
        if not media_image_id:
            return None, None
        image = await self.coordinator.client.image(media_image_id)
        return image if image else (None, None)

    async def async_search_media(self, query: SearchMediaQuery) -> SearchMedia:
        try:
            response = await self.coordinator.client.search(query.search_query)
        except MujinMusicError as error:
            raise HomeAssistantError(
                translation_domain=DOMAIN,
                translation_key="command_failed",
                translation_placeholders={"error": str(error)},
            ) from error
        results = [self._browse_item(item, "search") for item in response["children"]]
        if query.media_filter_classes:
            results = [
                item for item in results if item.media_class in query.media_filter_classes
            ]
        return SearchMedia(result=results)

    @callback
    def _handle_coordinator_update(self) -> None:
        zone, _ = self._resolve()
        if zone is not None and self.device_entry is not None:
            name = zone.get("name") if zone["id"] == self.zone_id else None
            if name and self.device_entry.name != name and not self.device_entry.name_by_user:
                dr.async_get(self.hass).async_update_device(self.device_entry.id, name=name)
        super()._handle_coordinator_update()

