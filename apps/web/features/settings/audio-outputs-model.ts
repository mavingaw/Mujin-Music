import {
  dsdRateLabel,
  DSD_STRATEGIES,
  formatSampleRate,
  networkCapsSourceLabels,
  type DsdStrategy,
  type FormatSupportEntry,
  type Zone,
  type ZoneEndpoint,
  type ZoneSettingsView,
} from "@parson/music-sdk";

/**
 * One output on the settings page: a zone the server plays itself and
 * the physical endpoint behind it. Its setup (Exclusive Mode, format and
 * DSD limits) is the zone's settings.
 */
export interface OutputEntry {
  zone: Zone;
  endpoint: ZoneEndpoint;
}

/** The settings an output's notes depend on. */
export type OutputSetup = Pick<
  ZoneSettingsView,
  "exclusive" | "dsd_strategy" | "max_sample_rate" | "max_bit_depth"
>;

/** How a row of the "Bit-Perfect Format Support" list is shown. */
export type FormatTone = "supported" | "capped" | "unsupported";

/** Unsupported (red) wins over capped by the device's max settings. */
export function formatTone(entry: FormatSupportEntry): FormatTone {
  if (!entry.supported) return "unsupported";
  return entry.allowed ? "supported" : "capped";
}

export function formatToneLabel(tone: FormatTone) {
  switch (tone) {
    case "supported":
      return "Bit-perfect";
    case "capped":
      return "Supported, above your limit: converted";
    case "unsupported":
      return "Not supported: converted";
  }
}

const APP_KINDS = new Set<Zone["kind"]>(["web", "desktop", "mobile"]);

/**
 * Every zone that plays through one endpoint of its own (a device, a
 * bridge's device, a network player), the server's own devices first.
 * Groups list their members' endpoints under the members' own zones.
 */
export function outputEntries(zones: readonly Zone[]) {
  const entries: OutputEntry[] = [];
  for (const zone of zones) {
    // Apps play through their own system mixer, groups through members.
    if (APP_KINDS.has(zone.kind) || zone.kind === "linked" || zone.group)
      continue;
    const endpoint = zone.endpoints?.find((item) => item.id === zone.id);
    if (endpoint) entries.push({ zone, endpoint });
  }
  return {
    local: entries.filter((entry) => !entry.endpoint.network),
    network: entries.filter((entry) => Boolean(entry.endpoint.network)),
  };
}

/** Choices for the max sample rate: every rate the device takes. */
export function maxRateOptions(endpoint: ZoneEndpoint) {
  return (endpoint.formats?.sample_rates ?? [])
    .filter((entry) => entry.supported)
    .map((entry) => ({
      value: entry.value,
      label: formatSampleRate(entry.value),
    }));
}

export function maxDepthOptions(endpoint: ZoneEndpoint) {
  return (endpoint.formats?.bit_depths ?? [])
    .filter((entry) => entry.supported)
    .map((entry) => ({ value: entry.value, label: `${entry.value} bit` }));
}

/** e.g. `44.1kHz to 192kHz, 16/24/32 bit, 2 channels`. */
export function deviceCapabilitySummary(endpoint: ZoneEndpoint) {
  if (!endpoint.formats) return endpoint.probe_error ?? "Formats unknown";
  const rates = endpoint.formats.sample_rates.filter(
    (entry) => entry.supported,
  );
  const depths = endpoint.formats.bit_depths
    .filter((entry) => entry.supported)
    .map((entry) => entry.value);
  if (!rates.length || !depths.length) return "No bit-perfect PCM formats";
  const first = formatSampleRate(rates[0]!.value);
  const last = formatSampleRate(rates.at(-1)!.value);
  const channels = endpoint.channels
    ? endpoint.channels[0] === endpoint.channels[1]
      ? `${endpoint.channels[0]} channels`
      : `${endpoint.channels[0]}–${endpoint.channels[1]} channels`
    : null;
  return [
    first === last ? first : `${first} to ${last}`,
    `${depths.join("/")} bit`,
    channels,
  ]
    .filter(Boolean)
    .join(", ");
}

/** Where this player's format limits come from, and what to do about it. */
export function networkLimitsNote(endpoint: ZoneEndpoint) {
  const network = endpoint.network;
  if (!network) return null;
  const source = networkCapsSourceLabels[network.caps_source];
  switch (network.caps_source) {
    case "declared":
      return `${source}: it lists the rates it takes, so Parson sends only those and converts the rest on the server.`;
    case "protocol":
      return `${source}: AirPlay 1 carries ALAC at 44.1kHz/16bit only, so higher-resolution files are converted on the server first, and the signal path shows it.`;
    case "known_model":
      return `${source} for ${network.model ?? endpoint.name}; Parson converts anything above them on the server.`;
    case "assumed":
      return `${source}. It takes WAV but does not list rates, so Parson allows up to 192kHz/24bit. If a high-resolution track does not play, lower Max sample rate in the zone's settings.`;
  }
}

/** How the audio travels to this player, in a sentence. */
export function networkStreamNote(endpoint: ZoneEndpoint) {
  const network = endpoint.network;
  if (!network) return null;
  if (network.protocol === "air_play")
    return "AirPlay 1 (RAOP): the server's chain output is sent as lossless ALAC frames over RTP, with timing and sync. Rated High quality at best: the receiver applies its own volume and Parson cannot see past it.";
  return `${network.stream_format}: the server runs its whole chain, DSP included, then serves the result losslessly and tells the player over ${network.protocol === "open_home" ? "OpenHome" : "UPnP AV"} to fetch it. Nothing on this computer mixes it.${network.reports_format ? " This player reports what it decodes, so the signal path can confirm the format." : ""}`;
}

/** DSD rates the device takes natively. */
export function nativeDsdRates(endpoint: ZoneEndpoint) {
  return (endpoint.formats?.dsd_rates ?? [])
    .filter((entry) => entry.supported)
    .map((entry) => entry.value);
}

/** e.g. `Native DSD64–DSD256 (DSD_U32_BE)`, `DoP possible up to DSD128`. */
export function dsdSupportSummary(endpoint: ZoneEndpoint) {
  if (!endpoint.formats) return null;
  const native = nativeDsdRates(endpoint);
  const dop = endpoint.formats.dop_rates ?? [];
  const span = (rates: number[]) =>
    rates.length === 1
      ? dsdRateLabel(rates[0]!)
      : `${dsdRateLabel(rates[0]!)}–${dsdRateLabel(rates.at(-1)!)}`;
  if (native.length) {
    const formats = endpoint.dsd_formats.length
      ? ` (${endpoint.dsd_formats.join(", ")})`
      : "";
    return `Native ${span(native)}${formats}`;
  }
  if (dop.length) return `No native DSD; DoP carrier fits ${span(dop)}`;
  return "No DSD: DSD files are converted to PCM";
}

/** Choices for the max DSD rate: rates the device can take as DSD. */
export function maxDsdRateOptions(endpoint: ZoneEndpoint) {
  const rates = new Set([
    ...nativeDsdRates(endpoint),
    ...(endpoint.formats?.dop_rates ?? []),
  ]);
  return [...rates]
    .sort((a, b) => a - b)
    .map((value) => ({ value, label: dsdRateLabel(value) }));
}

/** What the chosen strategy will do on this device, in a sentence. */
export function dsdStrategyNote(endpoint: ZoneEndpoint, setup: OutputSetup) {
  const strategy: DsdStrategy = setup.dsd_strategy ?? "auto";
  const native = nativeDsdRates(endpoint).length > 0;
  const dop = (endpoint.formats?.dop_rates ?? []).length > 0;
  const base =
    DSD_STRATEGIES.find((item) => item.value === strategy)?.description ?? "";
  if (!setup.exclusive)
    return "DSD needs Exclusive Mode; with it off, DSD files are converted to PCM.";
  if ((strategy === "native" || strategy === "auto") && !native)
    return `${base} The driver reports no native DSD, so DSD files are converted to PCM${dop ? "; choose DoP if the DAC decodes DSD over PCM" : ""}.`;
  if (strategy === "dop" && !dop)
    return "No 24-bit PCM rate here is high enough to carry DoP, so DSD files are converted to PCM.";
  if (strategy === "dop")
    return `${base} If the DAC plays noise, it does not decode DoP.`;
  return base;
}

/** e.g. `USB 21b4:0082 · high speed · formats probed by the driver`. */
export function hardwareSummary(endpoint: ZoneEndpoint) {
  const hardware = endpoint.hardware;
  if (!hardware) return null;
  return [
    hardware.bus.toUpperCase() + (hardware.usb_id ? ` ${hardware.usb_id}` : ""),
    hardware.speed,
    hardware.caps_source === "driver"
      ? "formats probed by the driver"
      : "device busy: formats from its USB descriptors",
    hardware.kernel_emulator
      ? "kernel ABI emulator, no DAC hardware attached"
      : null,
  ]
    .filter(Boolean)
    .join(" · ");
}

/** The USB descriptors' playback formats, one line each. */
export function descriptorLines(endpoint: ZoneEndpoint) {
  return (endpoint.hardware?.altsets ?? []).map((altset) => {
    const rates = altset.rates;
    const span = rates.length
      ? rates.length === 1
        ? formatSampleRate(rates[0]!)
        : `${formatSampleRate(rates[0]!)}–${formatSampleRate(rates.at(-1)!)}`
      : "rates unknown";
    return [
      altset.formats.join(" "),
      altset.bits ? `${altset.bits} bit` : null,
      altset.channels ? `${altset.channels}ch` : null,
      span,
    ]
      .filter(Boolean)
      .join(", ");
  });
}
