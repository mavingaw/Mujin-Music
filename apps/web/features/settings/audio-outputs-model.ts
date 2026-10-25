import {
  nativeFormatLabel,
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
