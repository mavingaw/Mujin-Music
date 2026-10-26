# Parson Bridge

Parson Bridge turns the audio outputs of another computer into Parson zones.
Run it on the machine your DAC is plugged into: a Raspberry Pi next to the
hi-fi, a NAS, a desktop. The Parson server finds it on the local network
(DNS-SD `_parson-bridge._tcp`), or you add it by address in the zone picker
(Zones > Bridges and network speakers > Add a bridge by address).

## How playback works

- The Parson server decodes each track at the file's own sample rate and bit
  depth, applies the zone's settings (volume leveling, DSP, software volume,
  only when you turn them on), and sends the samples to the bridge as
  uncompressed PCM over TCP (port 1994).
- The bridge opens the DAC exclusively (`hw:` on ALSA, no mixing or
  conversion) at exactly that format and writes the samples as they arrive.
  With hardware volume the DAC's own mixer changes the level, so the
  samples stay bit-perfect. The zone's signal path shows every step.
- Zones on bridges group natively: a group plays one stream on every
  member, sample-synced to the first zone's DAC clock (the clock master,
  which stays bit-perfect). The others follow it to within a quarter
  millisecond by repeating or skipping a single frame when their clocks
  drift. The server measures each bridge's clock offset over the network.

## Run it

```sh
parson-bridge --list            # show outputs and the formats they take
parson-bridge                   # serve on port 1994, advertise on the LAN
parson-bridge --name "Living Room" --port 1994
parson-bridge --simulated       # two simulated DACs, for trying it out
```

`packaging/parson-bridge.service` runs it as a systemd service. The
machine's firewall must allow TCP 1994 from the Parson server (and UDP 5353
for discovery).

## Platforms and what is tested

- Linux, including Raspberry Pi OS (arm64 and armhf): ALSA, loaded at run
  time. Tested with simulated DACs over real sockets and with ALSA's null
  device; not yet on a real USB DAC.
- Windows: WASAPI in exclusive mode, event-driven, at the file's own rate
  and depth (16, 24, or 24-in-32 bits as the DAC takes them), with the
  documented buffer-alignment retry. `--list` shows each endpoint's native
  rates, depths and hardware volume range in dB (only volume the DAC itself
  applies counts; Windows' software endpoint volume is bypassed by
  exclusive streams). A device with exclusive mode turned off in Windows
  sound settings plays shared, through the Windows mixer, and says it is
  not bit-perfect. Allow TCP 1994 in Windows Defender Firewall.
- macOS: CoreAudio in hog mode. The bridge takes the DAC for itself,
  switches its nominal sample rate to the file's (waiting for the change to
  land) and sets the stream's physical format to integer PCM at the file's
  depth. Samples pass the HAL's float stage, which carries 24 bits exactly,
  so 16- and 24-bit audio reaches the DAC bit-perfect; 32-bit integer
  sources are not offered. Hog mode is released when playback ends.
- Both are tested against protocol-level fakes of each audio stack (exact
  formats, buffer alignment, whole-buffer events and exclusive ownership
  for WASAPI; hog-mode toggling, asynchronous rate switching and the
  float-to-integer conversion for CoreAudio), end to end through a Parson
  server, and compile-checked for `x86_64-pc-windows-msvc` and
  `aarch64-apple-darwin`; not yet on a real Windows PC or Mac with a DAC.

Build: `cargo build --release -p parson-bridge` (cross-compile for a Pi with
`--target aarch64-unknown-linux-gnu`).
