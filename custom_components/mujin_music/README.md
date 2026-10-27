# Mujin Music for Home Assistant

Every Mujin Music zone becomes a Home Assistant media player: the web,
desktop and phone apps, Sonos rooms and groups, AirPlay and UPnP speakers,
audio devices of the server, Mujin Music Bridge players and linked groups.
Home Assistant talks to your server directly on your network. Nothing goes
through the cloud, and music keeps the quality your server sends each zone.

## Install

1. In HACS open **⋮ → Custom repositories**, add
   `https://github.com/mavingaw/Mujin-Music` as an **Integration**, download
   **Mujin Music** and restart Home Assistant.
2. Home Assistant finds the server by itself (it announces
   `_parson._tcp` on your network) and shows **Mujin Music discovered** under
   **Settings → Devices & services**. You can also add **Mujin Music** by hand
   with the server's address and port (1993 unless you changed it).
3. Choose **Pair with a code**. Home Assistant shows six digits: enter them
   in Mujin Music under **Settings → Home Assistant → Approve Home Assistant**
   and select **Approve**. Home Assistant gets its
   own revocable session for your account and never stores a password.
   Signing in with a username and password is available too.

## What you get

| Media player feature                             | How it works                                                                                                                                                                                                                        |
| ------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Play, pause, stop, next, previous                | Sent to the zone at once; the state comes back over the event stream.                                                                                                                                                               |
| Seek, shuffle, repeat (off/all/one), clear queue | Per zone.                                                                                                                                                                                                                           |
| Volume set, step, mute                           | Steps use the zone's own step size and respect its comfort and safety limits.                                                                                                                                                       |
| Grouped volume                                   | On a group the slider sets the group; each room keeps its own player and slider.                                                                                                                                                    |
| Join and unjoin                                  | Sonos rooms group natively (sample-synced). Other zones form a linked group, which shows as its own player while it exists.                                                                                                         |
| Transfer                                         | Pick **Play from …** as the source, or call `mujin_music.transfer`: the queue, track and position move to the other zone.                                                                                                           |
| Media browser                                    | Albums, artists, playlists, genres, liked songs and radio stations, with artwork. Long lists are split A–Z. Hi-res albums show their format, e.g. `FLAC 24/96`.                                                                     |
| Play media                                       | By browser id (`album:…`, `artist:…`, `playlist:…`, `genre:…`, `song:…`, `radio:…`) or by text: `media_content_type: artist` and `media_content_id: Miles Davis` plays the best match. Enqueue next, add and replace are supported. |
| Turn on, turn off                                | Zones that hold a device (audio devices, Bridge players, UPnP, AirPlay) go to standby: playback stops and the device is released, the queue stays. Turn on wakes the zone; play resumes where it stopped.                           |
| Search                                           | `media_player.search_media` searches artists, albums and songs.                                                                                                                                                                     |
| Now playing                                      | Title, artist, album, duration, position and full-quality artwork.                                                                                                                                                                  |
| Signal path                                      | `quality`, `bit_perfect`, `source_format`, `output_format` and `signal_path` attributes show what reaches the speaker.                                                                                                              |

Text that matches nothing in the library fails with a clear "Nothing in
the Mujin Music library matches …" message.

## Volume on your amplifier (volume requests)

Like Roon's volume control for Home Assistant: in the Mujin Music app open a
zone's settings and set **Home Assistant → Volume handled by** to **Home
Assistant**. The zone then plays at full level and every volume change made
anywhere (the apps, the media player, a remote) goes out as an event instead.
Its device gets an `event.<zone>_volume_request` entity whose event types
are `volume_up`, `volume_down`, `volume_set`, `mute_on` and `mute_off`, with
`level` (Mujin Music's running estimate, 0-100), `step` and `muted`.

```yaml
automation:
  - alias: Hi-Fi volume on the amplifier
    triggers:
      - trigger: state
        entity_id: event.hi_fi_volume_request
    actions:
      - choose:
          - conditions: "{{ trigger.to_state.attributes.event_type == 'volume_up' }}"
            sequence:
              - action: remote.send_command
                target:
                  entity_id: remote.living_room_ir
                data:
                  device: amplifier
                  command: volume_up
          - conditions: "{{ trigger.to_state.attributes.event_type == 'volume_down' }}"
            sequence:
              - action: remote.send_command
                target:
                  entity_id: remote.living_room_ir
                data:
                  device: amplifier
                  command: volume_down
```

Updates are pushed by the server the moment anything changes; a slow
fallback poll only keeps positions honest if the event stream drops.

## Actions

| Action                    | What it does                                                                                                             |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `mujin_music.start_radio` | Starts an endless station seeded by `item` (a browser id) or `query` (+ optional `kind`), or library radio with neither. |
| `mujin_music.thumbs_up`   | Likes the playing track; radio plays more like it.                                                                       |
| `mujin_music.thumbs_down` | Dislikes the playing track; it is skipped and radio stays away from it.                                                  |
| `mujin_music.transfer`    | Moves playback to `destination`.                                                                                         |
| `mujin_music.sleep_timer` | Fades out and pauses after `minutes` (0 cancels).                                                                        |
| `mujin_music.pause_all`   | Pauses every playing zone.                                                                                               |

```yaml
automation:
  - alias: Good morning music
    triggers:
      - trigger: time
        at: "07:30:00"
    actions:
      - action: mujin_music.start_radio
        target:
          entity_id: media_player.kitchen
        data:
          query: Morning Jazz
          kind: genre
```

## Development

The tests run Home Assistant against a fake Mujin Music server that speaks
the real HTTP and WebSocket API:

```sh
python3.13 -m venv .venv-ha
.venv-ha/bin/pip install -r tests/home_assistant/requirements.txt
.venv-ha/bin/python -m pytest -c tests/home_assistant/pytest.ini tests/home_assistant
```
