import {
  activeEqBands,
  crossfeedActive,
  crossfeedStrengthLabels,
  formatDecibels,
  formatRealtimeFactor,
  formatSampleRate,
  headroomActive,
  nativeFormatLabel,
  normalizeDspSettings,
  type NativeOutputDevice,
  type NativeOutputSession,
  type ServerDspStatus,
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

/**
 * What the server's DSP chain does on a device, e.g.
 * `Headroom −3.0 dB · EQ 2 bands · Crossfeed Medium · 96kHz`, or `null`
 * when it passes the samples through untouched.
 */
export function serverDspSummary(device: NativeOutputDevice): string | null {
  if (!device.dsp) return null;
  const dsp = normalizeDspSettings(device.dsp);
  if (!dsp.enabled) return null;
  const parts: string[] = [];
  if (headroomActive(dsp))
    parts.push(`Headroom ${formatDecibels(dsp.headroom.gainDb)}`);
  const bands = activeEqBands(dsp).length;
  if (bands) parts.push(`EQ ${bands} ${bands === 1 ? "band" : "bands"}`);
  if (crossfeedActive(dsp))
    parts.push(`Crossfeed ${crossfeedStrengthLabels[dsp.crossfeed.strength]}`);
  if (dsp.sampleRate.enabled && typeof dsp.sampleRate.mode === "number")
    parts.push(formatSampleRate(dsp.sampleRate.mode));
  return parts.length ? parts.join(" · ") : null;
}

/** e.g. `Processing speed 41.3x · Peak −1.2 dBFS · Clipping`. */
export function serverDspStatusLine(status: ServerDspStatus | null) {
  if (!status) return null;
  const parts: string[] = [];
  if (status.realtime_factor)
    parts.push(
      `Processing speed ${formatRealtimeFactor(status.realtime_factor)}`,
    );
  if (status.peak_dbfs !== null)
    parts.push(`Peak ${formatDecibels(status.peak_dbfs)}FS`);
  if (status.clipping) parts.push(`Clipping (${status.clipped_samples})`);
  return parts.length ? parts.join(" · ") : null;
}
