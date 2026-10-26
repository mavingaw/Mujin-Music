"""Volume requests from zones whose volume Home Assistant carries out.

A zone set to "Volume handled by: Home Assistant" in the Mujin Music app
leaves its audio at full level and sends every volume change out instead.
Each such zone gets an event entity here; an automation listens to it and
drives whatever really sets the volume (an amplifier on an IR blaster, an
AV receiver's own integration).
"""

from __future__ import annotations

from typing import Any

from homeassistant.components.event import EventEntity
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers import entity_registry as er
from homeassistant.helpers.dispatcher import async_dispatcher_connect
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity

from .const import (
    CONF_INSTANCE_ID,
    VOLUME_CONTROL_HOME_ASSISTANT,
    VOLUME_REQUEST_EVENT_TYPES,
    volume_request_signal,
)
from .coordinator import MujinMusicConfigEntry, MujinMusicCoordinator
from .device import zone_device_info

PARALLEL_UPDATES = 0

UNIQUE_ID_SUFFIX = "-volume_request"


def _delegated(zone: dict[str, Any] | None) -> bool:
    return (
        zone is not None
        and (zone.get("volume") or {}).get("control") == VOLUME_CONTROL_HOME_ASSISTANT
    )


async def async_setup_entry(
    hass: HomeAssistant,
    entry: MujinMusicConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    """Adds an entity for every zone that hands its volume to Home Assistant."""
    coordinator = entry.runtime_data
    instance_id = entry.data[CONF_INSTANCE_ID]
    entities: dict[str, VolumeRequestEvent] = {}

    # Entities from earlier sessions come back (unavailable until the zone
    # hands its volume over again), so automations keep their triggers.
    prefix = f"{instance_id}-"
    restored: list[VolumeRequestEvent] = []
    for registered in er.async_entries_for_config_entry(er.async_get(hass), entry.entry_id):
        unique_id = registered.unique_id
        if (
            registered.domain != "event"
            or not unique_id.startswith(prefix)
            or not unique_id.endswith(UNIQUE_ID_SUFFIX)
        ):
            continue
        zone_id = unique_id.removeprefix(prefix).removesuffix(UNIQUE_ID_SUFFIX)
        entities[zone_id] = VolumeRequestEvent(coordinator, instance_id, zone_id)
        restored.append(entities[zone_id])
    if restored:
        async_add_entities(restored)

    @callback
    def sync_entities() -> None:
        new: list[VolumeRequestEvent] = []
        for zone_id, zone in coordinator.data.zones.items():
            if zone_id in entities or not _delegated(zone):
                continue
            entities[zone_id] = VolumeRequestEvent(coordinator, instance_id, zone_id)
            new.append(entities[zone_id])
        if new:
            async_add_entities(new)

    sync_entities()
    entry.async_on_unload(coordinator.async_add_listener(sync_entities))


class VolumeRequestEvent(CoordinatorEntity[MujinMusicCoordinator], EventEntity):
    """Fires `volume_up`, `volume_down`, `volume_set`, `mute_on` or `mute_off`."""

    _attr_has_entity_name = True
    _attr_translation_key = "volume_request"
    _attr_event_types = list(VOLUME_REQUEST_EVENT_TYPES)

    def __init__(
        self, coordinator: MujinMusicCoordinator, instance_id: str, zone_id: str
    ) -> None:
        super().__init__(coordinator)
        self.zone_id = zone_id
        self._attr_unique_id = f"{instance_id}-{zone_id}{UNIQUE_ID_SUFFIX}"
        # The same device as the zone's media player.
        self._attr_device_info = zone_device_info(coordinator.data, instance_id, zone_id)

    @property
    def available(self) -> bool:
        return super().available and _delegated(self.coordinator.data.zone(self.zone_id))

    async def async_added_to_hass(self) -> None:
        await super().async_added_to_hass()
        self.async_on_remove(
            async_dispatcher_connect(
                self.hass,
                volume_request_signal(self.coordinator.config_entry.entry_id),
                self._handle_request,
            )
        )

    @callback
    def _handle_request(self, request: dict[str, Any]) -> None:
        if request.get("zone_id") != self.zone_id:
            return
        action = request.get("action")
        if action not in VOLUME_REQUEST_EVENT_TYPES:
            return
        self._trigger_event(
            action,
            {
                "level": request.get("level"),
                "step": request.get("step"),
                "muted": request.get("muted"),
            },
        )
        self.async_write_ha_state()
