import {
  dsdRateLabel,
  DSD_STRATEGIES,
  nativeFormatLabel,
  type DsdStrategy,
  type FormatSupportEntry,
  type NativeOutputDevice,
} from "@parson/music-sdk";

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

/** Choices for the max sample rate: every rate the device takes. */
export function maxRateOptions(device: NativeOutputDevice) {
  return (device.formats?.sample_rates ?? [])
    .filter((entry) => entry.supported)
    .map((entry) => ({
      value: entry.value,
      label: nativeFormatLabel(entry.value),
    }));
}

export function maxDepthOptions(device: NativeOutputDevice) {
  return (device.formats?.bit_depths ?? [])
    .filter((entry) => entry.supported)
    .map((entry) => ({ value: entry.value, label: `${entry.value} bit` }));
}

/** e.g. `44.1kHz to 192kHz, 16/24/32 bit, 2 channels`. */
export function deviceCapabilitySummary(device: NativeOutputDevice) {
  if (!device.formats) return device.probe_error ?? "Formats unknown";
  const rates = device.formats.sample_rates.filter((entry) => entry.supported);
  const depths = device.formats.bit_depths
    .filter((entry) => entry.supported)
    .map((entry) => entry.value);
  if (!rates.length || !depths.length) return "No bit-perfect PCM formats";
  const first = nativeFormatLabel(rates[0]!.value);
  const last = nativeFormatLabel(rates.at(-1)!.value);
  const channels = device.channels
    ? device.channels[0] === device.channels[1]
      ? `${device.channels[0]} channels`
      : `${device.channels[0]}–${device.channels[1]} channels`
    : null;
  return [
    first === last ? first : `${first} to ${last}`,
    `${depths.join("/")} bit`,
    channels,
  ]
    .filter(Boolean)
    .join(", ");
}

/** Driver names as the settings page shows them. */
export const driverLabels: Record<NativeOutputDevice["driver"], string> = {
  alsa: "ALSA",
  wasapi: "WASAPI",
  core_audio: "CoreAudio",
  simulated: "Simulated",
};

/** DSD rates the device takes natively. */
export function nativeDsdRates(device: NativeOutputDevice) {
  return (device.formats?.dsd_rates ?? [])
    .filter((entry) => entry.supported)
    .map((entry) => entry.value);
}

/** e.g. `Native DSD64–DSD256 (DSD_U32_BE)`, `DoP possible up to DSD128`. */
export function dsdSupportSummary(device: NativeOutputDevice) {
  if (!device.formats) return null;
  const native = nativeDsdRates(device);
  const dop = device.formats.dop_rates ?? [];
  const span = (rates: number[]) =>
    rates.length === 1
      ? dsdRateLabel(rates[0]!)
      : `${dsdRateLabel(rates[0]!)}–${dsdRateLabel(rates.at(-1)!)}`;
  if (native.length) {
    const formats = device.dsd_formats?.length
      ? ` (${device.dsd_formats.join(", ")})`
      : "";
    return `Native ${span(native)}${formats}`;
  }
  if (dop.length) return `No native DSD; DoP carrier fits ${span(dop)}`;
  return "No DSD: DSD files are converted to PCM";
}

/** Choices for the max DSD rate: rates the device can take as DSD. */
export function maxDsdRateOptions(device: NativeOutputDevice) {
  const rates = new Set([
    ...nativeDsdRates(device),
    ...(device.formats?.dop_rates ?? []),
  ]);
  return [...rates]
    .sort((a, b) => a - b)
    .map((value) => ({ value, label: dsdRateLabel(value) }));
}

/** What the chosen strategy will do on this device, in a sentence. */
export function dsdStrategyNote(device: NativeOutputDevice) {
  const strategy: DsdStrategy = device.settings.dsd_strategy ?? "auto";
  const native = nativeDsdRates(device).length > 0;
  const dop = (device.formats?.dop_rates ?? []).length > 0;
  const base =
    DSD_STRATEGIES.find((item) => item.value === strategy)?.description ?? "";
  if (!device.settings.exclusive)
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
export function hardwareSummary(device: NativeOutputDevice) {
  const hardware = device.hardware;
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
export function descriptorLines(device: NativeOutputDevice) {
  return (device.hardware?.altsets ?? []).map((altset) => {
    const rates = altset.rates;
    const span = rates.length
      ? rates.length === 1
        ? nativeFormatLabel(rates[0]!)
        : `${nativeFormatLabel(rates[0]!)}–${nativeFormatLabel(rates.at(-1)!)}`
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
