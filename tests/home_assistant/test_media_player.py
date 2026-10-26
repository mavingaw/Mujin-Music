"""Media players against the fake server: state, push, transport, volume,
grouping, transfer, browse, play_media, search, radio and thumbs."""

from __future__ import annotations

from http import HTTPStatus

import pytest
from pytest_homeassistant_custom_component.common import MockConfigEntry
from pytest_homeassistant_custom_component.typing import ClientSessionGenerator

from custom_components.mujin_music.const import CONF_ACCESS_TOKEN, DOMAIN
from homeassistant.components.media_player import (
    ATTR_GROUP_MEMBERS,
    ATTR_INPUT_SOURCE,
    ATTR_INPUT_SOURCE_LIST,
    ATTR_MEDIA_ALBUM_NAME,
    ATTR_MEDIA_ARTIST,
    ATTR_MEDIA_CONTENT_ID,
    ATTR_MEDIA_CONTENT_TYPE,
    ATTR_MEDIA_DURATION,
    ATTR_MEDIA_ENQUEUE,
    ATTR_MEDIA_POSITION,
    ATTR_MEDIA_REPEAT,
    ATTR_MEDIA_SEEK_POSITION,
    ATTR_MEDIA_SHUFFLE,
    ATTR_MEDIA_TITLE,
    ATTR_MEDIA_VOLUME_LEVEL,
    ATTR_MEDIA_VOLUME_MUTED,
    DOMAIN as MP_DOMAIN,
    SERVICE_CLEAR_PLAYLIST,
    SERVICE_JOIN,
    SERVICE_PLAY_MEDIA,
    SERVICE_SELECT_SOURCE,
    SERVICE_UNJOIN,
    MediaPlayerEnqueue,
    MediaPlayerEntityFeature,
)
from homeassistant.const import (
    ATTR_ENTITY_ID,
    SERVICE_MEDIA_NEXT_TRACK,
    SERVICE_MEDIA_PAUSE,
    SERVICE_MEDIA_PLAY,
    SERVICE_MEDIA_SEEK,
    SERVICE_MEDIA_STOP,
    SERVICE_REPEAT_SET,
    SERVICE_SHUFFLE_SET,
    SERVICE_VOLUME_DOWN,
    SERVICE_VOLUME_MUTE,
    SERVICE_VOLUME_SET,
    SERVICE_VOLUME_UP,
    STATE_IDLE,
    STATE_PAUSED,
    STATE_PLAYING,
    STATE_UNAVAILABLE,
)
from homeassistant.core import HomeAssistant
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers import device_registry as dr, entity_registry as er
from homeassistant.setup import async_setup_component

from .fake_server import COVER_BYTES, INSTANCE_ID, FakeMujinServer, settle

STUDIO = "media_player.studio"
KITCHEN = "media_player.kitchen"
PATIO = "media_player.patio"


async def call(hass: HomeAssistant, service: str, entity_id: str, **data) -> None:
    await hass.services.async_call(
        MP_DOMAIN, service, {ATTR_ENTITY_ID: entity_id, **data}, blocking=True
    )


async def test_zones_become_players_with_now_playing(
    hass: HomeAssistant, fake_server: FakeMujinServer, loaded_entry: MockConfigEntry
) -> None:
    state = hass.states.get(STUDIO)
    assert state.state == STATE_PLAYING
    assert state.attributes[ATTR_MEDIA_TITLE] == "Nocturne No. 1"
    assert state.attributes[ATTR_MEDIA_ARTIST] == "Clara Keys"
    assert state.attributes[ATTR_MEDIA_ALBUM_NAME] == "Nocturnes"
    assert state.attributes[ATTR_MEDIA_CONTENT_ID] == "song:noc-1"
    assert state.attributes[ATTR_MEDIA_DURATION] == 245
    assert state.attributes[ATTR_MEDIA_POSITION] == 61
    assert state.attributes[ATTR_MEDIA_VOLUME_LEVEL] == 0.42
    # The signal path rides along: lossless, bit-perfect.
    assert state.attributes["quality"] == "lossless"
    assert state.attributes["bit_perfect"] is True
    assert state.attributes["source_format"] == "FLAC 24/96"
    assert state.attributes["queue_size"] == 2
    assert state.attributes["entity_picture"].startswith("/api/media_player_proxy/")
    features = MediaPlayerEntityFeature(state.attributes["supported_features"])
    for feature in (
        MediaPlayerEntityFeature.PLAY,
        MediaPlayerEntityFeature.SEEK,
        MediaPlayerEntityFeature.VOLUME_STEP,
        MediaPlayerEntityFeature.GROUPING,
        MediaPlayerEntityFeature.BROWSE_MEDIA,
        MediaPlayerEntityFeature.SEARCH_MEDIA,
        MediaPlayerEntityFeature.MEDIA_ENQUEUE,
        MediaPlayerEntityFeature.SHUFFLE_SET,
        MediaPlayerEntityFeature.REPEAT_SET,
    ):
        assert feature in features
    assert hass.states.get(KITCHEN).state == STATE_IDLE

    device = dr.async_get(hass).async_get_device(
        identifiers={(DOMAIN, f"{INSTANCE_ID}-sonos:RINCON_KITCHEN")}
    )
    assert device.manufacturer == "Sonos"
    assert device.model == "Sonos Era 300"
    hub = dr.async_get(hass).async_get_device(identifiers={(DOMAIN, INSTANCE_ID)})
    assert device.via_device_id == hub.id


async def test_artwork_is_fetched_with_the_session(
    hass: HomeAssistant,
    hass_client: ClientSessionGenerator,
    fake_server: FakeMujinServer,
    loaded_entry: MockConfigEntry,
) -> None:
    await async_setup_component(hass, "http", {})
    client = await hass_client()
    picture = hass.states.get(STUDIO).attributes["entity_picture"]
    response = await client.get(picture)
    assert response.status == HTTPStatus.OK
    assert await response.read() == COVER_BYTES
    assert fake_server.calls_to("cover.jpg")[-1] == {"raw": "true"}


async def test_changes_are_pushed_without_polling(
    hass: HomeAssistant, fake_server: FakeMujinServer, loaded_entry: MockConfigEntry
) -> None:
    fake_server.zones["sonos:RINCON_KITCHEN"]["status"] = "playing"
    fake_server.zones["sonos:RINCON_KITCHEN"]["volume"]["level"] = 15
    await fake_server.push()
    await settle(hass)
    assert hass.states.get(KITCHEN).state == STATE_PLAYING
    assert hass.states.get(KITCHEN).attributes[ATTR_MEDIA_VOLUME_LEVEL] == 0.15

    # A zone that goes away (a phone asleep) stays, unavailable.
    del fake_server.zones["sonos:RINCON_PATIO"]
    await fake_server.push()
    await settle(hass)
    assert hass.states.get(PATIO).state == STATE_UNAVAILABLE


async def test_transport_volume_shuffle_repeat(
    hass: HomeAssistant, fake_server: FakeMujinServer, loaded_entry: MockConfigEntry
) -> None:
    await call(hass, SERVICE_MEDIA_PAUSE, STUDIO)
    assert hass.states.get(STUDIO).state == STATE_PAUSED
    await call(hass, SERVICE_MEDIA_PLAY, STUDIO)
    await call(hass, SERVICE_MEDIA_NEXT_TRACK, STUDIO)
    await call(hass, SERVICE_MEDIA_SEEK, STUDIO, **{ATTR_MEDIA_SEEK_POSITION: 90.4})
    await call(hass, SERVICE_VOLUME_SET, STUDIO, **{ATTR_MEDIA_VOLUME_LEVEL: 0.5})
    await call(hass, SERVICE_VOLUME_UP, STUDIO)
    await call(hass, SERVICE_VOLUME_DOWN, KITCHEN)
    await call(hass, SERVICE_VOLUME_MUTE, STUDIO, **{ATTR_MEDIA_VOLUME_MUTED: True})
    await call(hass, SERVICE_SHUFFLE_SET, STUDIO, **{ATTR_MEDIA_SHUFFLE: True})
    await call(hass, SERVICE_REPEAT_SET, STUDIO, **{ATTR_MEDIA_REPEAT: "one"})
    await call(hass, SERVICE_CLEAR_PLAYLIST, STUDIO)
    await call(hass, SERVICE_MEDIA_STOP, STUDIO)
    assert fake_server.calls_to("/client:desktop-studio1/commands") == [
        {"command": "pause"},
        {"command": "play"},
        {"command": "next"},
        {"command": "seek", "position_ms": 90400},
        {"command": "set_volume", "volume": 50},
        {"command": "adjust_volume", "delta": 4},
        {"command": "set_mute", "muted": True},
        {"command": "set_shuffle", "enabled": True},
        {"command": "set_repeat", "mode": "one"},
        {"command": "clear_upcoming"},
        {"command": "stop"},
    ]
    assert fake_server.calls_to("/sonos:RINCON_KITCHEN/commands") == [
        {"command": "adjust_volume", "delta": -4}
    ]
    await settle(hass)
    state = hass.states.get(STUDIO)
    assert state.attributes[ATTR_MEDIA_VOLUME_LEVEL] == 0.54
    assert state.attributes[ATTR_MEDIA_VOLUME_MUTED] is True
    assert state.attributes[ATTR_MEDIA_SHUFFLE] is True
    assert state.attributes[ATTR_MEDIA_REPEAT] == "one"


async def test_sonos_grouping_with_per_room_and_group_volume(
    hass: HomeAssistant, fake_server: FakeMujinServer, loaded_entry: MockConfigEntry
) -> None:
    await call(hass, SERVICE_JOIN, KITCHEN, **{ATTR_GROUP_MEMBERS: [PATIO]})
    assert fake_server.calls_to("/zones/groups") == [
        {"zone_ids": ["sonos:RINCON_KITCHEN", "sonos:RINCON_PATIO"]}
    ]
    await settle(hass)
    kitchen = hass.states.get(KITCHEN)
    patio = hass.states.get(PATIO)
    # The Patio folded into the Kitchen's group but keeps its player.
    assert kitchen.attributes[ATTR_GROUP_MEMBERS] == [KITCHEN, PATIO]
    assert patio.attributes[ATTR_GROUP_MEMBERS] == [KITCHEN, PATIO]
    assert patio.state == kitchen.state
    assert patio.attributes["playing_in_group"] == "Kitchen + Patio"

    # Group volume on the coordinator, room volume on the member.
    await call(hass, SERVICE_VOLUME_SET, KITCHEN, **{ATTR_MEDIA_VOLUME_LEVEL: 0.2})
    await call(hass, SERVICE_VOLUME_SET, PATIO, **{ATTR_MEDIA_VOLUME_LEVEL: 0.6})
    assert fake_server.calls_to("/sonos:RINCON_KITCHEN/commands") == [
        {"command": "set_volume", "volume": 20},
        {"command": "set_volume", "volume": 60, "member_id": "sonos:RINCON_PATIO"},
    ]
    await settle(hass)
    assert hass.states.get(PATIO).attributes[ATTR_MEDIA_VOLUME_LEVEL] == 0.6

    await call(hass, SERVICE_UNJOIN, PATIO)
    assert fake_server.calls[-1] == (
        "DELETE",
        "/api/v1/zones/groups/sonos:RINCON_KITCHEN",
        {"member": "sonos:RINCON_PATIO"},
    )
    await settle(hass)
    assert ATTR_GROUP_MEMBERS not in hass.states.get(PATIO).attributes or not hass.states.get(
        PATIO
    ).attributes[ATTR_GROUP_MEMBERS]


async def test_linked_group_appears_and_is_removed(
    hass: HomeAssistant, fake_server: FakeMujinServer, loaded_entry: MockConfigEntry
) -> None:
    await call(hass, SERVICE_JOIN, STUDIO, **{ATTR_GROUP_MEMBERS: [KITCHEN]})
    await settle(hass)
    linked = er.async_get(hass).async_get_entity_id(
        MP_DOMAIN, DOMAIN, f"{INSTANCE_ID}-linked:5b1f"
    )
    assert linked is not None
    assert hass.states.get(STUDIO).attributes[ATTR_GROUP_MEMBERS] == [STUDIO, KITCHEN]
    assert hass.states.get(linked).attributes["group_synced"] is False

    await call(hass, SERVICE_UNJOIN, linked)
    await settle(hass)
    assert (
        er.async_get(hass).async_get_entity_id(MP_DOMAIN, DOMAIN, f"{INSTANCE_ID}-linked:5b1f")
        is None
    )
    assert hass.states.get(linked) is None


async def test_transfer_by_source_and_by_service(
    hass: HomeAssistant, fake_server: FakeMujinServer, loaded_entry: MockConfigEntry
) -> None:
    kitchen = hass.states.get(KITCHEN)
    assert kitchen.attributes[ATTR_INPUT_SOURCE_LIST] == ["Play from Studio"]
    await call(hass, SERVICE_SELECT_SOURCE, KITCHEN, **{ATTR_INPUT_SOURCE: "Play from Studio"})
    assert fake_server.calls_to("/zones/transfer") == [
        {"from": "client:desktop-studio1", "to": "sonos:RINCON_KITCHEN"}
    ]
    await settle(hass)
    assert hass.states.get(KITCHEN).attributes[ATTR_MEDIA_TITLE] == "Nocturne No. 1"
    assert hass.states.get(STUDIO).state == STATE_IDLE

    await hass.services.async_call(
        DOMAIN,
        "transfer",
        {ATTR_ENTITY_ID: KITCHEN, "destination": PATIO},
        blocking=True,
    )
    assert fake_server.calls_to("/zones/transfer")[-1] == {
        "from": "sonos:RINCON_KITCHEN",
        "to": "sonos:RINCON_PATIO",
    }
    with pytest.raises(HomeAssistantError):
        await call(hass, SERVICE_SELECT_SOURCE, KITCHEN, **{ATTR_INPUT_SOURCE: "Nowhere"})


async def test_play_media_by_id_search_and_enqueue(
    hass: HomeAssistant, fake_server: FakeMujinServer, loaded_entry: MockConfigEntry
) -> None:
    await call(
        hass,
        SERVICE_PLAY_MEDIA,
        KITCHEN,
        **{ATTR_MEDIA_CONTENT_TYPE: "album", ATTR_MEDIA_CONTENT_ID: "album:album-nocturnes"},
    )
    await call(
        hass,
        SERVICE_PLAY_MEDIA,
        KITCHEN,
        **{
            ATTR_MEDIA_CONTENT_TYPE: "track",
            ATTR_MEDIA_CONTENT_ID: "song:noc-2",
            ATTR_MEDIA_ENQUEUE: MediaPlayerEnqueue.NEXT,
        },
    )
    await call(
        hass,
        SERVICE_PLAY_MEDIA,
        KITCHEN,
        **{ATTR_MEDIA_CONTENT_TYPE: "artist", ATTR_MEDIA_CONTENT_ID: "Clara Keys"},
    )
    await call(
        hass,
        SERVICE_PLAY_MEDIA,
        KITCHEN,
        **{
            ATTR_MEDIA_CONTENT_TYPE: "music",
            ATTR_MEDIA_CONTENT_ID: "nocturne",
            ATTR_MEDIA_ENQUEUE: MediaPlayerEnqueue.ADD,
        },
    )
    assert fake_server.calls_to("/sonos:RINCON_KITCHEN/play") == [
        {"action": "play", "item": "album:album-nocturnes"},
        {"action": "next", "item": "song:noc-2"},
        {"action": "play", "query": "Clara Keys", "kind": "artist"},
        {"action": "add", "query": "nocturne"},
    ]
    await settle(hass)
    assert hass.states.get(KITCHEN).state == STATE_PLAYING

    with pytest.raises(HomeAssistantError, match="zone not found"):
        await call(
            hass,
            SERVICE_PLAY_MEDIA,
            KITCHEN,
            **{ATTR_MEDIA_CONTENT_TYPE: "music", ATTR_MEDIA_CONTENT_ID: "nothing like this"},
        )
    with pytest.raises(HomeAssistantError):
        await call(
            hass,
            SERVICE_PLAY_MEDIA,
            KITCHEN,
            **{
                ATTR_MEDIA_CONTENT_TYPE: "music",
                ATTR_MEDIA_CONTENT_ID: "media-source://tts/abc",
            },
        )


async def test_browse_and_search_the_library(
    hass: HomeAssistant,
    hass_ws_client,
    fake_server: FakeMujinServer,
    loaded_entry: MockConfigEntry,
) -> None:
    client = await hass_ws_client(hass)
    await client.send_json_auto_id(
        {"type": "media_player/browse_media", "entity_id": STUDIO}
    )
    root = (await client.receive_json())["result"]
    assert root["title"] == "Mujin Music · Home Library"
    assert [child["media_content_id"] for child in root["children"]] == [
        "albums",
        "artists",
        "playlists",
        "genres",
        "liked",
        "radio",
    ]
    liked = root["children"][4]
    assert liked["can_play"] is True

    await client.send_json_auto_id(
        {
            "type": "media_player/browse_media",
            "entity_id": STUDIO,
            "media_content_type": "library",
            "media_content_id": "albums",
        }
    )
    albums = (await client.receive_json())["result"]
    titles = [child["title"] for child in albums["children"]]
    # Hi-res albums carry their format; lossy ones do not.
    assert titles == ["Nocturnes · Clara Keys · FLAC 24/96", "The Neon Drive · Synth Club"]
    assert albums["children"][0]["media_class"] == "album"
    assert albums["children"][0]["thumbnail"].startswith(
        f"/api/media_player_proxy/{STUDIO}/browse_media/album/album:album-nocturnes?"
    )
    assert albums["children"][1]["thumbnail"] is None
    assert albums["children_media_class"] == "album"

    await client.send_json_auto_id(
        {
            "type": "media_player/browse_media",
            "entity_id": STUDIO,
            "media_content_type": "album",
            "media_content_id": "album:album-nocturnes",
        }
    )
    album = (await client.receive_json())["result"]
    assert [child["title"] for child in album["children"]] == [
        "Nocturne No. 1",
        "Nocturne No. 2",
    ]
    assert album["children"][0]["media_class"] == "track"

    await client.send_json_auto_id(
        {
            "type": "media_player/browse_media",
            "entity_id": STUDIO,
            "media_content_type": "radio",
            "media_content_id": "radio",
        }
    )
    radio = (await client.receive_json())["result"]
    assert [child["media_class"] for child in radio["children"]] == ["channel", "channel"]

    await client.send_json_auto_id(
        {
            "type": "media_player/browse_media",
            "entity_id": STUDIO,
            "media_content_type": "album",
            "media_content_id": "album:missing",
        }
    )
    assert (await client.receive_json())["success"] is False

    await client.send_json_auto_id(
        {
            "type": "media_player/search_media",
            "entity_id": STUDIO,
            "search_query": "nocturne",
        }
    )
    found = (await client.receive_json())["result"]["result"]
    assert [item["media_content_id"] for item in found] == [
        "album:album-nocturnes",
        "song:noc-1",
        "song:noc-2",
    ]
    await client.send_json_auto_id(
        {
            "type": "media_player/search_media",
            "entity_id": STUDIO,
            "search_query": "nocturne",
            "media_filter_classes": ["track"],
        }
    )
    tracks = (await client.receive_json())["result"]["result"]
    assert [item["media_class"] for item in tracks] == ["track", "track"]


async def test_radio_thumbs_sleep_and_pause_all(
    hass: HomeAssistant, fake_server: FakeMujinServer, loaded_entry: MockConfigEntry
) -> None:
    response = await hass.services.async_call(
        DOMAIN,
        "start_radio",
        {ATTR_ENTITY_ID: KITCHEN, "query": "jazz", "kind": "genre"},
        blocking=True,
        return_response=True,
    )
    assert response == {KITCHEN: {"station": "Jazz radio", "songs": 61}}
    await hass.services.async_call(
        DOMAIN, "start_radio", {ATTR_ENTITY_ID: STUDIO}, blocking=True
    )
    assert fake_server.calls_to("/radio") == [
        {"action": "radio", "query": "jazz", "kind": "genre"},
        {"action": "radio"},
    ]
    # Radio stations from the browser play as radio.
    await call(
        hass,
        SERVICE_PLAY_MEDIA,
        STUDIO,
        **{ATTR_MEDIA_CONTENT_TYPE: "channel", ATTR_MEDIA_CONTENT_ID: "radio:genre:Jazz"},
    )
    assert fake_server.calls_to("/client:desktop-studio1/play")[-1] == {
        "action": "play",
        "item": "radio:genre:Jazz",
    }

    await hass.services.async_call(DOMAIN, "thumbs_up", {ATTR_ENTITY_ID: STUDIO}, blocking=True)
    await hass.services.async_call(DOMAIN, "thumbs_down", {ATTR_ENTITY_ID: STUDIO}, blocking=True)
    assert fake_server.calls_to("/rating") == [{"rating": "up"}, {"rating": "down"}]

    await hass.services.async_call(
        DOMAIN, "sleep_timer", {ATTR_ENTITY_ID: STUDIO, "minutes": 30}, blocking=True
    )
    await hass.services.async_call(
        DOMAIN, "sleep_timer", {ATTR_ENTITY_ID: STUDIO, "minutes": 0}, blocking=True
    )
    assert fake_server.calls_to("/sleep") == [{"minutes": 30}, {"minutes": None}]

    paused = await hass.services.async_call(
        DOMAIN, "pause_all", {}, blocking=True, return_response=True
    )
    assert paused == {"paused": 1}
    await settle(hass)
    assert hass.states.get(STUDIO).state == STATE_PAUSED


async def test_expired_access_token_is_renewed_and_saved(
    hass: HomeAssistant, fake_server: FakeMujinServer, loaded_entry: MockConfigEntry
) -> None:
    fake_server.access_token = "rotated-away"
    await call(hass, SERVICE_MEDIA_PAUSE, STUDIO)
    assert fake_server.calls_to("/auth/refresh") == [None]
    assert loaded_entry.data[CONF_ACCESS_TOKEN] == fake_server.access_token
    assert hass.states.get(STUDIO).state == STATE_PAUSED


async def test_unload_closes_the_event_stream(
    hass: HomeAssistant, fake_server: FakeMujinServer, config_entry: MockConfigEntry
) -> None:
    config_entry.add_to_hass(hass)
    assert await hass.config_entries.async_setup(config_entry.entry_id)
    await settle(hass)
    assert len(fake_server.sockets) == 1
    assert await hass.config_entries.async_unload(config_entry.entry_id)
    await settle(hass)
    assert fake_server.sockets == []
