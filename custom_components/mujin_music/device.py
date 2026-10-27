"""The device a zone's entities share (its media player and, when the zone
hands its volume to Home Assistant, its volume request events)."""

from __future__ import annotations

from homeassistant.helpers.device_registry import DeviceInfo

from .const import DOMAIN, MANUFACTURER
from .coordinator import ZoneSnapshot

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


def zone_device_info(snapshot: ZoneSnapshot, instance_id: str, zone_id: str) -> DeviceInfo:
    """Name and model from the zone, or from its entry in a group it plays in."""
    zone = snapshot.zone(zone_id)
    name = (zone or {}).get("name")
    if not name and (group := snapshot.group_containing(zone_id)) is not None:
        name = next(
            (
                member.get("name")
                for member in group["group"].get("members", [])
                if member.get("id") == zone_id
            ),
            None,
        )
    kind = (zone or {}).get("kind", "")
    return DeviceInfo(
        identifiers={(DOMAIN, f"{instance_id}-{zone_id}")},
        name=name or zone_id,
        manufacturer="Sonos" if kind == "sonos" else MANUFACTURER,
        model=(zone or {}).get("device") or KIND_LABEL.get(kind, "Zone"),
        via_device=(DOMAIN, instance_id),
    )
