"""Constants for the Mujin Music integration."""

from __future__ import annotations

from typing import Final

DOMAIN: Final = "mujin_music"
BRAND: Final = "Mujin Music"
MANUFACTURER: Final = "Mujin Music"

# The server announces itself with this DNS-SD type (the wire protocol keeps
# its original name, so every server version is found).
ZEROCONF_TYPE: Final = "_parson._tcp.local."
PRODUCT_ID: Final = "parson-music"
DEFAULT_PORT: Final = 1993

CONF_INSTANCE_ID: Final = "instance_id"
CONF_ACCESS_TOKEN: Final = "access_token"
CONF_REFRESH_TOKEN: Final = "refresh_token"
CONF_USE_SSL: Final = "ssl"

API_PREFIX: Final = "/api/v1"
NATIVE_CLIENT_HEADER: Final = "x-parson-client"

# Push first; this slow poll only covers a server that cannot open the
# event stream (and keeps position drift honest).
FALLBACK_POLL_SECONDS: Final = 60
# Several events in a burst (a track change touches volume, queue and
# status) refresh once.
EVENT_DEBOUNCE_SECONDS: Final = 0.15
RECONNECT_MIN_SECONDS: Final = 1
RECONNECT_MAX_SECONDS: Final = 60

SERVICE_START_RADIO: Final = "start_radio"
SERVICE_THUMBS_UP: Final = "thumbs_up"
SERVICE_THUMBS_DOWN: Final = "thumbs_down"
SERVICE_TRANSFER: Final = "transfer"
SERVICE_SLEEP_TIMER: Final = "sleep_timer"
SERVICE_PAUSE_ALL: Final = "pause_all"

ATTR_ITEM: Final = "item"
ATTR_QUERY: Final = "query"
ATTR_KIND: Final = "kind"
ATTR_DESTINATION: Final = "destination"
ATTR_MINUTES: Final = "minutes"

SEARCH_KINDS: Final = ("artist", "album", "song", "playlist", "genre")
