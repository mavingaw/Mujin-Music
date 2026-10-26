import {
  activeConvolutionFilter,
  activeEqBands,
  crossfeedActive,
  crossfeedStrengthLabels,
  dsdRateLabel,
  DSD_STRATEGIES,
  formatDecibels,
  formatRealtimeFactor,
  formatSampleRate,
  headroomActive,
  networkCapsSourceLabels,
  networkOutputIdentity,
  normalizeDspSettings,
  type DsdStrategy,
  type DspSettings,
  type ServerDspStatus,
  type SignalPath,
  type Zone,
  type ZoneEndpoint,
} from "@parson/music-sdk";

/**
 * A zone the server plays itself (a device, a bridge's device, a network
 * player) and the physical endpoint behind it.
 */
export interface ServerOutput {
  zone: Zone;
  endpoint: ZoneEndpoint;
}

const APP_KINDS = new Set<Zone["kind"]>(["web", "desktop", "mobile"]);

/** Zones that play through an endpoint of their own; groups are skipped. */
export function serverOutputs(zones: readonly Zone[]): ServerOutput[] {
  const outputs: ServerOutput[] = [];
  for (const zone of zones) {
    // Apps play through their own system mixer, groups through members.
    if (APP_KINDS.has(zone.kind) || zone.kind === "linked" || zone.group)
      continue;
    const endpoint = zone.endpoints?.find((item) => item.id === zone.id);
    if (endpoint) outputs.push({ zone, endpoint });
  }
  return outputs;
}

/**
 * e.g. `Bit-perfect 44.1kHz–192kHz, 16/24 bit`; for a network player
 * `Up to 192kHz, 16/24 bit (assumed)` or `44.1kHz, 16 bit (AirPlay)`.
 */
export function serverOutputSummary(endpoint: ZoneEndpoint) {
  if (!endpoint.formats) return endpoint.probe_error ?? "Formats unknown";
  const rates = endpoint.formats.sample_rates.filter(
    (entry) => entry.supported,
  );
  const depths = endpoint.formats.bit_depths
    .filter((entry) => entry.supported)
    .map((entry) => entry.value);
  if (endpoint.network) {
    if (!rates.length || !depths.length) return "No PCM formats";
    const first = formatSampleRate(rates[0]!.value);
    const last = formatSampleRate(rates.at(-1)!.value);
    const span = first === last ? first : `${first}–${last}`;
    const source =
      endpoint.network.caps_source === "protocol"
        ? "AirPlay"
        : endpoint.network.caps_source === "assumed"
          ? "assumed"
          : endpoint.network.caps_source === "known_model"
            ? "maker's limits"
            : "declared";
    return `${span}, ${depths.join("/")} bit (${source})`;
  }
  if (!rates.length || !depths.length) return "No bit-perfect PCM formats";
  const first = formatSampleRate(rates[0]!.value);
  const last = formatSampleRate(rates.at(-1)!.value);
  return `Bit-perfect ${first === last ? first : `${first}–${last}`}, ${depths.join("/")} bit`;
}

/** Rates the device cannot play bit-perfectly, e.g. `352.8kHz, 384kHz`. */
export function unsupportedRates(endpoint: ZoneEndpoint) {
  return (endpoint.formats?.sample_rates ?? [])
    .filter((entry) => !entry.supported)
    .map((entry) => formatSampleRate(entry.value))
    .join(", ");
}

/** `Playing · m:ss / m:ss` for what a server zone plays. */
export function serverSessionLine(
  zone: Pick<Zone, "status" | "position_ms" | "duration_ms">,
) {
  const clock = (ms: number) => {
    const seconds = Math.max(0, Math.floor(ms / 1000));
    return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
  };
  const state =
    zone.status === "playing" || zone.status === "buffering"
      ? "Playing"
      : zone.status === "paused"
        ? "Paused"
        : "Stopped";
  return `${state} · ${clock(zone.position_ms)} / ${clock(zone.duration_ms)}`;
}

/**
 * What a zone's DSP chain does, e.g.
 * `Headroom −3.0 dB · EQ 2 bands · Crossfeed Medium · 96kHz`, or `null`
 * when it passes the samples through untouched.
 */
export function serverDspSummary(
  settings: DspSettings | null | undefined,
): string | null {
  if (!settings) return null;
  const dsp = normalizeDspSettings(settings);
  if (!dsp.enabled) return null;
  const parts: string[] = [];
  if (headroomActive(dsp))
    parts.push(`Headroom ${formatDecibels(dsp.headroom.gainDb)}`);
  const bands = activeEqBands(dsp).length;
  if (bands) parts.push(`EQ ${bands} ${bands === 1 ? "band" : "bands"}`);
  if (crossfeedActive(dsp))
    parts.push(`Crossfeed ${crossfeedStrengthLabels[dsp.crossfeed.strength]}`);
  if (activeConvolutionFilter(dsp)) parts.push("Convolution");
  if (dsp.sampleRate.enabled && typeof dsp.sampleRate.mode === "number")
    parts.push(formatSampleRate(dsp.sampleRate.mode));
  return parts.length ? parts.join(" · ") : null;
}

/** e.g. `Linn Klimax DSM · OpenHome · 192.168.1.40`, or `null` off-network. */
export function networkPlayerLine(endpoint: ZoneEndpoint) {
  const network = endpoint.network;
  if (!network) return null;
  return `${networkOutputIdentity(network)} · ${network.host}`;
}

/** Where a network player's limits come from and what it was tested on. */
export function networkPlayerNote(endpoint: ZoneEndpoint) {
  const network = endpoint.network;
  if (!network) return null;
  return `${networkCapsSourceLabels[network.caps_source]}. ${
    network.protocol === "air_play"
      ? "Tested against Parson's protocol-level AirPlay receiver, not yet a real AirPlay device."
      : "Tested against Parson's protocol-level UPnP/OpenHome renderer, not yet a real streamer."
  }`;
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
export function serverDsdLine(
  endpoint: ZoneEndpoint,
  strategy: DsdStrategy | undefined,
) {
  const entries = endpoint.formats?.dsd_rates;
  if (!entries) return null;
  const native = entries.filter((entry) => entry.supported).map((e) => e.value);
  const dop = endpoint.formats?.dop_rates ?? [];
  const span = (rates: number[]) =>
    rates.length === 1
      ? dsdRateLabel(rates[0]!)
      : `${dsdRateLabel(rates[0]!)}–${dsdRateLabel(rates.at(-1)!)}`;
  const support = native.length
    ? `native ${span(native)}`
    : dop.length
      ? `DoP up to ${dsdRateLabel(dop.at(-1)!)}`
      : "converted to PCM";
  const label =
    DSD_STRATEGIES.find((item) => item.value === (strategy ?? "auto"))?.label ??
    "Automatic";
  return `DSD: ${support} · ${label}`;
}

/** e.g. `USB 21b4:0082 · high speed`. */
export function serverHardwareLine(endpoint: ZoneEndpoint) {
  const hardware = endpoint.hardware;
  if (!hardware) return null;
  return [
    `${hardware.bus.toUpperCase()}${hardware.usb_id ? ` ${hardware.usb_id}` : ""}`,
    hardware.speed,
    hardware.caps_source === "usb_descriptors"
      ? "formats from USB descriptors"
      : null,
    hardware.kernel_emulator ? "kernel ABI emulator, no DAC hardware" : null,
  ]
    .filter(Boolean)
    .join(" · ");
}

/**
 * The driver's (or renderer's) report while the server plays, e.g.
 * `Confirmed by the driver: S24_3LE 96kHz 2ch`, or `null` without one.
 */
export function serverDriverLine(
  path: Pick<
    SignalPath,
    "hardwareReport" | "hardwareVerified" | "hardwareReporter"
  >,
) {
  if (!path.hardwareReport) return null;
  const reporter = path.hardwareReporter ?? "driver";
  if (path.hardwareVerified)
    return `Confirmed by the ${reporter}: ${path.hardwareReport}`;
  return reporter === "renderer"
    ? `Renderer: ${path.hardwareReport}, converted by the renderer`
    : `Driver: ${path.hardwareReport}, converted after Parson`;
}

/**
 * The server output as the kernel and driver name it, e.g.
 * `USB 152a:8750 · high speed · ALSA hw:1,0 exclusive on studio-pc`.
 */
export function serverTransportLine(path: Pick<SignalPath, "outputDevice">) {
  const device = path.outputDevice;
  if (!device) return null;
  if (device.network)
    return `${device.network.protocol_label} to ${device.network.endpoint} · ${device.network.stream_format} from ${device.host}`;
  return [
    device.usb_id ? `USB ${device.usb_id}` : null,
    device.usb_speed,
    `${device.transport} on ${device.host}`,
    device.kernel_emulator ? "kernel ABI emulator, no DAC hardware" : null,
  ]
    .filter(Boolean)
    .join(" · ");
}
