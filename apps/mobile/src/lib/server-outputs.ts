import {
  activeEqBands,
  crossfeedActive,
  crossfeedStrengthLabels,
  dsdRateLabel,
  DSD_STRATEGIES,
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

/** e.g. `DSD: native DSD64–DSD256 · Automatic`, or `null` without DSD info. */
export function serverDsdLine(device: NativeOutputDevice) {
  const entries = device.formats?.dsd_rates;
  if (!entries) return null;
  const native = entries.filter((entry) => entry.supported).map((e) => e.value);
  const dop = device.formats?.dop_rates ?? [];
  const span = (rates: number[]) =>
    rates.length === 1
      ? dsdRateLabel(rates[0]!)
      : `${dsdRateLabel(rates[0]!)}–${dsdRateLabel(rates.at(-1)!)}`;
  const support = native.length
    ? `native ${span(native)}`
    : dop.length
      ? `DoP up to ${dsdRateLabel(dop.at(-1)!)}`
      : "converted to PCM";
  const strategy =
    DSD_STRATEGIES.find(
      (item) => item.value === (device.settings.dsd_strategy ?? "auto"),
    )?.label ?? "Automatic";
  return `DSD: ${support} · ${strategy}`;
}

/** e.g. `USB 21b4:0082 · AudioQuest DragonFly Red v1.0`. */
export function serverHardwareLine(device: NativeOutputDevice) {
  const hardware = device.hardware;
  if (!hardware) return null;
  return [
    `${hardware.bus.toUpperCase()}${hardware.usb_id ? ` ${hardware.usb_id}` : ""}`,
    hardware.speed,
    hardware.caps_source === "usb_descriptors"
      ? "formats from USB descriptors"
      : null,
  ]
    .filter(Boolean)
    .join(" · ");
}

/**
 * The driver's report while the server plays, e.g. `Confirmed by the
 * driver: hardware running at S24_3LE 96kHz 2ch`, or `null` when there is
 * none.
 */
export function serverDriverLine(session: NativeOutputSession) {
  const hardware = session.hardware;
  if (!hardware) return null;
  const params = hardware.params;
  const running = `${params.format} ${formatSampleRate(params.rate)} ${params.channels}ch`;
  const chain = hardware.chain?.plugins.length
    ? ` via ${hardware.chain.plugins.join(" → ")}`
    : "";
  if (hardware.matches_stream) {
    return `Confirmed by the driver: ${hardware.reporter === "library" ? "running" : "hardware running"} at ${running}${chain}`;
  }
  const converting = hardware.chain?.converting.length
    ? ` by ${hardware.chain.converting.join(", ")}`
    : "";
  return `Driver: ${running}, converted after Parson${converting}`;
}

/**
 * The server output as the kernel and driver name it, e.g.
 * `USB 152a:8750 · high speed · ALSA hw:1,0 exclusive on studio-pc`.
 */
export function serverTransportLine(session: NativeOutputSession) {
  const device = session.signal_path?.output_device;
  if (!device) return null;
  return [
    device.usb_id ? `USB ${device.usb_id}` : null,
    device.usb_speed,
    `${device.transport} on ${device.host}`,
  ]
    .filter(Boolean)
    .join(" · ");
}
