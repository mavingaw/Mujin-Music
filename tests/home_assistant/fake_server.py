"""A fake Mujin Music server speaking the real HTTP and WebSocket API.

It keeps zones in memory, applies commands the way the server does (enough
for the integration to see the effect), checks bearer tokens, rotates
single-use refresh tokens and pushes `zones_changed` over the event stream.
"""

from __future__ import annotations

import asyncio
import copy
import itertools
from typing import Any

from aiohttp import WSMsgType, web
from aiohttp.test_utils import TestServer

INSTANCE_ID = "8f6c1d0e2b7a4c39a1d2e3f405162738"
COVER_BYTES = b"\x89PNG\r\n\x1a\nfake-cover"


def _zone(
    zone_id: str,
    name: str,
    kind: str,
    *,
    status: str = "idle",
    volume: int = 30,
    song: dict[str, Any] | None = None,
) -> dict[str, Any]:
    return {
        "id": zone_id,
        "kind": kind,
        "ecosystem": "sonos" if kind == "sonos" else "parson",
        "name": name,
        "icon": "speaker",
        "device": "Sonos Era 300" if kind == "sonos" else "Mujin Music for Linux",
        "status": status,
        "source": "parson" if song else "none",
        "now_playing": song,
        "queue": {
            "song_ids": [song["song_id"], "song-2"] if song else [],
            "current_position": 0,
        },
        "repeat": "off",
        "shuffle": False,
        "position_ms": 61_000 if song else 0,
        "duration_ms": song["duration_ms"] if song else 0,
        "volume": {
            "level": volume,
            "muted": False,
            "min": 0,
            "max": 100,
            "comfort": 70,
            "device_min": 0,
            "device_max": 100,
            "step": 4,
        },
        "group": None,
        "grouped_into": None,
        "capabilities": {
            "transport": True,
            "seek": True,
            "volume": True,
            "mute": True,
            "queue": True,
            "queue_edit": True,
            "repeat": True,
            "shuffle": True,
            "group": True,
            "transfer_target": True,
            "sleep_timer": True,
        },
        "sleep_timer": None,
        "settings": {},
        "output": {
            "max_sample_rate": 192_000,
            "max_bit_depth": 24,
            "formats": ["flac"],
            "note": "",
            "stream": {
                "source": "FLAC 24/96",
                "output": "FLAC 24/96",
                "bit_perfect": True,
                "steps": ["Decoded FLAC 24/96", "Sent unchanged"],
                "quality": "lossless",
            },
        }
        if song
        else None,
        "notes": [],
    }


NOCTURNE = {
    "song_id": "noc-1",
    "title": "Nocturne No. 1",
    "artist": "Clara Keys",
    "album": "Nocturnes",
    "album_id": "album-nocturnes",
    "cover_url": "/music/Clara Keys/Nocturnes/cover.jpg",
    "duration_ms": 245_000,
}

BROWSE: dict[str, dict[str, Any]] = {
    "root": {
        "item": {
            "id": "root",
            "title": "Home Library",
            "kind": "library",
            "playable": False,
            "expandable": True,
        },
        "children": [
            {"id": "albums", "title": "Albums", "kind": "directory", "playable": False, "expandable": True},
            {"id": "artists", "title": "Artists", "kind": "directory", "playable": False, "expandable": True},
            {"id": "playlists", "title": "Playlists", "kind": "directory", "playable": False, "expandable": True},
            {"id": "genres", "title": "Genres", "kind": "directory", "playable": False, "expandable": True},
            {"id": "liked", "title": "Liked songs", "kind": "directory", "playable": True, "expandable": True},
            {"id": "radio", "title": "Radio", "kind": "radio", "playable": False, "expandable": True},
        ],
    },
    "albums": {
        "item": {"id": "albums", "title": "Albums", "kind": "directory", "playable": False, "expandable": True},
        "children": [
            {
                "id": "album:album-nocturnes",
                "title": "Nocturnes",
                "subtitle": "Clara Keys",
                "image": "/music/Clara Keys/Nocturnes/cover.jpg",
                "kind": "album",
                "playable": True,
                "expandable": True,
                "quality": "FLAC 24/96",
            },
            {
                "id": "album:album-neon",
                "title": "The Neon Drive",
                "subtitle": "Synth Club",
                "kind": "album",
                "playable": True,
                "expandable": True,
                "quality": "MP3 320",
            },
        ],
    },
    "album:album-nocturnes": {
        "item": {
            "id": "album:album-nocturnes",
            "title": "Nocturnes",
            "subtitle": "Clara Keys",
            "image": "/music/Clara Keys/Nocturnes/cover.jpg",
            "kind": "album",
            "playable": True,
            "expandable": True,
            "quality": "FLAC 24/96",
        },
        "children": [
            {
                "id": "song:noc-1",
                "title": "Nocturne No. 1",
                "subtitle": "Clara Keys",
                "kind": "song",
                "playable": True,
                "expandable": False,
                "duration_ms": 245000,
            },
            {
                "id": "song:noc-2",
                "title": "Nocturne No. 2",
                "subtitle": "Clara Keys",
                "kind": "song",
                "playable": True,
                "expandable": False,
                "duration_ms": 201000,
            },
        ],
    },
    "radio": {
        "item": {"id": "radio", "title": "Radio", "kind": "radio", "playable": False, "expandable": True},
        "children": [
            {"id": "radio:library", "title": "Library radio", "kind": "radio", "playable": True, "expandable": False},
            {"id": "radio:genre:Jazz", "title": "Jazz radio", "kind": "radio", "playable": True, "expandable": False},
        ],
    },
}


class FakeMujinServer:
    """In-memory server state plus the aiohttp app that serves it."""

    def __init__(self) -> None:
        self.username = "listener"
        self.password = "correct horse"
        self._token_ids = itertools.count(1)
        self.access_token = ""
        self.refresh_token = ""
        self.issue_tokens()
        self.calls: list[tuple[str, str, Any]] = []
        self.sockets: list[web.WebSocketResponse] = []
        self.pairing_approved = False
        self.pairing_code = "482913"
        self.zones: dict[str, dict[str, Any]] = {
            "client:desktop-studio1": _zone(
                "client:desktop-studio1",
                "Studio",
                "desktop",
                status="playing",
                volume=42,
                song=dict(NOCTURNE),
            ),
            "sonos:RINCON_KITCHEN": _zone("sonos:RINCON_KITCHEN", "Kitchen", "sonos"),
            "sonos:RINCON_PATIO": _zone("sonos:RINCON_PATIO", "Patio", "sonos"),
        }
        self.server: TestServer | None = None

    def issue_tokens(self) -> tuple[str, str]:
        number = next(self._token_ids)
        self.access_token = f"access-{number}"
        self.refresh_token = f"refresh-{number}"
        return self.access_token, self.refresh_token

    @property
    def port(self) -> int:
        assert self.server is not None
        return self.server.port

    async def start(self) -> None:
        app = web.Application()
        app.router.add_get("/.well-known/parson", self.manifest)
        app.router.add_post("/api/v1/auth/login", self.login)
        app.router.add_post("/api/v1/auth/refresh", self.refresh)
        app.router.add_post("/api/v1/auth/pairing/start", self.pairing_start)
        app.router.add_post("/api/v1/auth/pairing/status", self.pairing_status)
        app.router.add_get("/api/v1/zones", self.list_zones)
        app.router.add_get("/api/v1/zones/events", self.events)
        app.router.add_post("/api/v1/zones/transfer", self.transfer)
        app.router.add_post("/api/v1/zones/groups", self.group)
        app.router.add_delete("/api/v1/zones/groups/{group_id}", self.ungroup)
        app.router.add_post("/api/v1/zones/pause-all", self.pause_all)
        app.router.add_post("/api/v1/zones/{zone_id}/commands", self.command)
        app.router.add_post("/api/v1/zones/{zone_id}/play", self.play)
        app.router.add_post("/api/v1/zones/{zone_id}/radio", self.radio)
        app.router.add_post("/api/v1/zones/{zone_id}/rating", self.rating)
        app.router.add_post("/api/v1/zones/{zone_id}/sleep", self.sleep)
        app.router.add_get("/api/v1/browse", self.browse)
        app.router.add_get("/media/images/{path:.*}", self.image)
        self.server = TestServer(app, host="127.0.0.1")
        await self.server.start_server()

    async def close(self) -> None:
        for socket in list(self.sockets):
            await socket.close()
        if self.server is not None:
            await self.server.close()

    # Helpers -------------------------------------------------------------

    def _authorized(self, request: web.Request) -> bool:
        return request.headers.get("Authorization") == f"Bearer {self.access_token}"

    def _unauthorized(self) -> web.Response:
        return web.json_response({"error": "session_required"}, status=401)

    async def _record(self, request: web.Request) -> Any:
        body = await request.json() if request.can_read_body else None
        self.calls.append((request.method, request.path, body))
        return body

    def calls_to(self, suffix: str) -> list[Any]:
        return [body for _, path, body in self.calls if path.endswith(suffix)]

    async def push(self) -> None:
        for socket in list(self.sockets):
            if not socket.closed:
                await socket.send_json({"type": "zones_changed"})

    # Routes --------------------------------------------------------------

    async def manifest(self, request: web.Request) -> web.Response:
        return web.json_response(
            {
                "protocol": "parson",
                "protocolVersion": 1,
                "instanceId": INSTANCE_ID,
                "name": "Home Library",
                "product": "parson-music",
                "serverVersion": "1.0.1",
                "pairingRequired": True,
                "capabilities": ["zones", "browse", "radio", "ratings", "home_assistant"],
            }
        )

    def _auth_payload(self) -> dict[str, Any]:
        access, refresh = self.issue_tokens()
        return {
            "status": True,
            "access_token": access,
            "refresh_token": refresh,
            "message": None,
        }

    async def login(self, request: web.Request) -> web.Response:
        body = await request.json()
        if request.headers.get("x-parson-client") != "native":
            return web.json_response({"status": False}, status=400)
        if body.get("username") != self.username or body.get("password") != self.password:
            return web.json_response(
                {"status": False, "access_token": "", "refresh_token": "", "message": "Invalid credentials"},
                status=401,
            )
        return web.json_response(self._auth_payload())

    async def refresh(self, request: web.Request) -> web.Response:
        self.calls.append(("POST", request.path, None))
        if request.headers.get("Authorization") != f"Bearer {self.refresh_token}":
            return web.json_response({"status": False, "message": "Refresh session is invalid"}, status=401)
        return web.json_response(self._auth_payload())

    async def pairing_start(self, request: web.Request) -> web.Response:
        body = await request.json()
        assert body["device_name"] == "Home Assistant"
        return web.json_response(
            {"pairingId": "pair-1", "secret": "s3cret", "code": self.pairing_code, "expiresIn": 180}
        )

    async def pairing_status(self, request: web.Request) -> web.Response:
        body = await request.json()
        if body != {"pairing_id": "pair-1", "secret": "s3cret"}:
            return web.json_response({"status": False}, status=404)
        if not self.pairing_approved:
            return web.json_response({"status": False, "pending": True}, status=202)
        return web.json_response(self._auth_payload())

    async def list_zones(self, request: web.Request) -> web.Response:
        if not self._authorized(request):
            return self._unauthorized()
        return web.json_response(
            {"zones": copy.deepcopy(list(self.zones.values())), "server_time_ms": 0}
        )

    async def events(self, request: web.Request) -> web.StreamResponse:
        if not self._authorized(request):
            return self._unauthorized()
        socket = web.WebSocketResponse()
        await socket.prepare(request)
        self.sockets.append(socket)
        await socket.send_json({"type": "ready"})
        try:
            async for message in socket:
                if message.type == WSMsgType.ERROR:
                    break
        finally:
            self.sockets.remove(socket)
        return socket

    async def command(self, request: web.Request) -> web.Response:
        if not self._authorized(request):
            return self._unauthorized()
        body = await self._record(request)
        zone_id = request.match_info["zone_id"]
        zone = self.zones.get(zone_id)
        if zone is None:
            return web.json_response({"error": "zone_not_found"}, status=404)
        command = body["command"]
        target = zone
        if member := body.get("member_id"):
            target = next(m for m in zone["group"]["members"] if m["id"] == member)
        if command == "play":
            zone["status"] = "playing"
        elif command == "pause":
            zone["status"] = "paused"
        elif command == "stop":
            zone["status"] = "stopped"
        elif command == "seek":
            zone["position_ms"] = body["position_ms"]
        elif command == "set_volume":
            target["volume"]["level"] = body["volume"]
        elif command == "adjust_volume":
            target["volume"]["level"] = max(0, min(100, target["volume"]["level"] + body["delta"]))
        elif command == "set_mute":
            target["volume"]["muted"] = body["muted"]
        elif command == "set_shuffle":
            zone["shuffle"] = body["enabled"]
        elif command == "set_repeat":
            zone["repeat"] = body["mode"]
        elif command == "clear_upcoming":
            zone["queue"]["song_ids"] = zone["queue"]["song_ids"][:1]
        elif command == "set_volume_invalid":
            return web.json_response({"error": "invalid_volume"}, status=400)
        await self.push()
        return web.json_response({"accepted": True}, status=202)

    async def transfer(self, request: web.Request) -> web.Response:
        if not self._authorized(request):
            return self._unauthorized()
        body = await self._record(request)
        source, destination = self.zones[body["from"]], self.zones[body["to"]]
        for key in ("now_playing", "queue", "position_ms", "duration_ms", "output", "source"):
            destination[key] = copy.deepcopy(source[key])
        destination["status"] = "playing"
        source.update(status="stopped", now_playing=None, source="none")
        source["queue"] = {"song_ids": [], "current_position": 0}
        await self.push()
        return web.json_response({"zone": destination})

    async def group(self, request: web.Request) -> web.Response:
        if not self._authorized(request):
            return self._unauthorized()
        body = await self._record(request)
        ids = body["zone_ids"]
        if len(ids) < 2:
            return web.json_response({"error": "group_needs_two_to_sixteen_zones"}, status=400)
        if not all(zone_id.startswith("sonos:") for zone_id in ids):
            group_id = "linked:5b1f"
            members = [self.zones[zone_id] for zone_id in ids]
            linked = _zone(group_id, " + ".join(m["name"] for m in members), "linked")
            linked["group"] = {
                "id": group_id,
                "synced": False,
                "sync_note": "Linked zones are not sample-synced.",
                "members": [
                    {"id": m["id"], "name": m["name"], "kind": m["kind"], "volume": m["volume"]}
                    for m in members
                ],
            }
            for member in members:
                member["grouped_into"] = group_id
            self.zones[group_id] = linked
            await self.push()
            return web.json_response({"group_id": group_id, "synced": False}, status=201)
        # Sonos: members fold into the coordinator's zone.
        coordinator = self.zones[ids[0]]
        members = [coordinator] + [self.zones.pop(zone_id) for zone_id in ids[1:]]
        coordinator["name"] = " + ".join(m["name"] for m in members)
        coordinator["group"] = {
            "id": coordinator["id"],
            "synced": True,
            "sync_note": "Sonos plays grouped rooms in sync.",
            "members": [
                {
                    "id": m["id"],
                    "name": m["name"].split(" + ")[0],
                    "kind": "sonos",
                    "volume": copy.deepcopy(m["volume"]),
                }
                for m in members
            ],
        }
        await self.push()
        return web.json_response({"group_id": coordinator["id"], "synced": True}, status=201)

    async def ungroup(self, request: web.Request) -> web.Response:
        if not self._authorized(request):
            return self._unauthorized()
        self.calls.append(("DELETE", request.path, dict(request.query)))
        group_id = request.match_info["group_id"]
        member = request.query.get("member")
        group_zone = self.zones.get(group_id)
        if group_zone is None:
            return web.json_response({"error": "zone_not_found"}, status=404)
        if group_id.startswith("linked:"):
            for zone in self.zones.values():
                if zone.get("grouped_into") == group_id and member in (None, zone["id"]):
                    zone["grouped_into"] = None
            del self.zones[group_id]
        else:
            leaving = [m for m in group_zone["group"]["members"] if m["id"] == member]
            for left in leaving:
                self.zones[left["id"]] = _zone(left["id"], left["name"], "sonos")
            group_zone["group"]["members"] = [
                m for m in group_zone["group"]["members"] if m["id"] != member
            ]
            if len(group_zone["group"]["members"]) <= 1:
                group_zone["group"] = None
                group_zone["name"] = group_zone["name"].split(" + ")[0]
        await self.push()
        return web.Response(status=204)

    async def pause_all(self, request: web.Request) -> web.Response:
        if not self._authorized(request):
            return self._unauthorized()
        await self._record(request)
        paused = 0
        for zone in self.zones.values():
            if zone["status"] == "playing":
                zone["status"] = "paused"
                paused += 1
        await self.push()
        return web.json_response({"paused": paused})

    async def play(self, request: web.Request) -> web.Response:
        if not self._authorized(request):
            return self._unauthorized()
        body = await self._record(request)
        if body.get("query") == "nothing like this":
            return web.json_response({"error": "zone_not_found"}, status=404)
        zone = self.zones[request.match_info["zone_id"]]
        zone["status"] = "playing"
        zone["now_playing"] = dict(NOCTURNE)
        zone["source"] = "parson"
        await self.push()
        return web.json_response(
            {
                "zone_id": zone["id"],
                "item_id": body.get("item") or "album:album-nocturnes",
                "title": "Nocturnes",
                "action": body.get("action", "play"),
                "songs": 3,
            }
        )

    async def radio(self, request: web.Request) -> web.Response:
        if not self._authorized(request):
            return self._unauthorized()
        body = await self._record(request)
        await self.push()
        return web.json_response(
            {
                "zone_id": request.match_info["zone_id"],
                "item_id": body.get("item") or "radio:library",
                "title": "Jazz radio" if body.get("query") else "Library radio",
                "action": "radio",
                "songs": 61,
            }
        )

    async def rating(self, request: web.Request) -> web.Response:
        if not self._authorized(request):
            return self._unauthorized()
        body = await self._record(request)
        return web.json_response(
            {
                "zone_id": request.match_info["zone_id"],
                "song_id": "noc-1",
                "rating": body["rating"],
                "skipped": body["rating"] == "down",
            }
        )

    async def sleep(self, request: web.Request) -> web.Response:
        if not self._authorized(request):
            return self._unauthorized()
        await self._record(request)
        return web.Response(status=204)

    async def browse(self, request: web.Request) -> web.Response:
        if not self._authorized(request):
            return self._unauthorized()
        self.calls.append(("GET", request.path, dict(request.query)))
        if query := request.query.get("query"):
            children = [
                child
                for child in BROWSE["albums"]["children"] + BROWSE["album:album-nocturnes"]["children"]
                if query.lower() in child["title"].lower()
            ]
            return web.json_response(
                {
                    "item": {"id": "search", "title": f"Results for {query}", "kind": "directory", "playable": False, "expandable": True},
                    "children": children,
                    "total": len(children),
                }
            )
        item = request.query.get("item", "root")
        if item not in BROWSE:
            return web.json_response({"error": "zone_not_found"}, status=404)
        page = BROWSE[item]
        return web.json_response({**page, "total": len(page["children"])})

    async def image(self, request: web.Request) -> web.Response:
        if not self._authorized(request):
            return self._unauthorized()
        self.calls.append(("GET", request.path, dict(request.query)))
        if request.match_info["path"] != NOCTURNE["cover_url"]:
            return web.Response(status=404)
        return web.Response(body=COVER_BYTES, content_type="image/png")


async def settle(hass: Any) -> None:
    """Lets the push debounce fire and the refresh land."""
    await asyncio.sleep(0.3)
    await hass.async_block_till_done()
