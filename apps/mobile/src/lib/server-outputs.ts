import {
  nativeFormatLabel,
  type NativeOutputDevice,
  type NativeOutputSession,
} from "@parson/music-sdk";

/** e.g. `Bit-perfect 44.1kHz–192kHz, 16/24 bit`. */
export function serverOutputSummary(device: NativeOutputDevice) {
  if (!device.formats) return device.probe_error ?? "Formats unknown";
  const rates = device.formats.sample_rates.filter((entry) => entry.supported);
  const depths = device.formats.bit_depths
    .filter((entry) => entry.supported)
    .map((entry) => entry.value);
  if (!rates.length || !depths.length) return "No bit-perfect PCM formats";
  const first = nativeFormatLabel(rates[0]!.value);
  const last = nativeFormatLabel(rates.at(-1)!.value);
  return `Bit-perfect ${first === last ? first : `${first}–${last}`}, ${depths.join("/")} bit`;
}

/** Rates the device cannot play bit-perfectly, e.g. `352.8kHz, 384kHz`. */
export function unsupportedRates(device: NativeOutputDevice) {
  return (device.formats?.sample_rates ?? [])
    .filter((entry) => !entry.supported)
    .map((entry) => nativeFormatLabel(entry.value))
    .join(", ");
}

/** `m:ss` position line for the server session. */
export function serverSessionLine(session: NativeOutputSession) {
  const clock = (ms: number) => {
    const seconds = Math.max(0, Math.floor(ms / 1000));
    return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
  };
  const state =
    session.state === "playing" || session.state === "starting"
      ? "Playing"
      : session.state === "paused"
        ? "Paused"
        : session.state === "ended"
          ? "Finished"
          : session.state === "failed"
            ? (session.error ?? "Failed")
            : "Stopped";
  return `${state} · ${clock(session.position_ms)} / ${clock(session.duration_ms)}`;
}
