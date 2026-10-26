import api, { getApiBaseURL, isApiError } from "../core/http";
import type { DspSettings } from "../player/dsp";
import type { ServerSignalPath } from "../player/signal-path";

/** How the server drives a device. */
export type NativeOutputDriver =
  | "alsa"
  | "wasapi"
  | "core_audio"
  | "simulated"
  /** A UPnP AV or OpenHome renderer on the network. */
  | "upnp"
  /** An AirPlay receiver (AirPlay 1 / RAOP). */
  | "airplay";

/** The network drivers: the server streams to a player on the LAN. */
export function isNetworkDriver(driver: NativeOutputDriver | null | undefined) {
  return driver === "upnp" || driver === "airplay";
}

/** How Parson reaches a network player. */
export type NetworkProtocol = "upnp_av" | "open_home" | "air_play";

export const networkProtocolLabels: Record<NetworkProtocol, string> = {
  upnp_av: "UPnP AV",
  open_home: "OpenHome",
  air_play: "AirPlay",
};

/**
 * Where a network player's format limits come from: declared by the
 * renderer, fixed by the protocol (AirPlay), the maker's published limits,
 * or assumed by Parson because the renderer does not say.
 */
export type NetworkCapsSource =
  "declared" | "protocol" | "known_model" | "assumed";

export const networkCapsSourceLabels: Record<NetworkCapsSource, string> = {
  declared: "Declared by the player",
  protocol: "Fixed by the protocol",
  known_model: "Maker's published limits",
  assumed: "Assumed by Parson (the player does not say)",
};

export type NetworkEndpointKind =
  "streamer" | "speaker" | "television" | "receiver" | "computer";

/** A network player's protocol, identity and limits. */
export interface NetworkOutputInfo {
  protocol: NetworkProtocol;
  host: string;
  port: number;
  /** UPnP: the device description URL. */
  location: string | null;
  manufacturer: string | null;
  /** e.g. `Klimax DSM` or `HomePod mini`. */
  model: string | null;
  model_description: string | null;
  /** The player's own picture (fetch it through `nativeOutputIconUrl`). */
  icon_url: string | null;
  kind: NetworkEndpointKind;
  caps_source: NetworkCapsSource;
  /** e.g. `WAV (LPCM) over HTTP`. */
  stream_format: string;
  /** What the player says it accepts, e.g. `WAV`, `L16 44.1/48kHz`. */
  accepts: string[];
  /** The player reports what it decodes while playing (OpenHome). */
  reports_format: boolean;
  /** Added by address rather than discovered. */
  manual: boolean;
  /** What this output has, and has not, been tested against. */
  verification: string;
}

/** A network player added by address. */
export interface ManualNetworkEndpoint {
  protocol: NetworkProtocol;
  /** UPnP: a description URL or `host[:port]`; AirPlay: `host[:port]`. */
  address: string;
  name?: string | null;
}

export interface NetworkOutputsView {
  /** The server can play to network players. */
  available: boolean;
  /** SSDP and mDNS discovery is on. */
  discovery: boolean;
  manual: ManualNetworkEndpoint[];
}

export type NativeSampleFormat =
  | "s16"
  | "s24_packed"
  | "s24_in32"
  | "s32"
  | "dsd_u8"
  | "dsd_u16_le"
  | "dsd_u16_be"
  | "dsd_u32_le"
  | "dsd_u32_be";

/**
 * How DSD files reach the device (Roon's "DSD Playback Strategy").
 * `auto` sends native DSD when the driver reports it and converts to PCM
 * otherwise; DoP is never guessed, because a DAC that does not decode it
 * plays the carrier as noise.
 */
export type DsdStrategy = "auto" | "native" | "dop" | "pcm";

export const DSD_STRATEGIES: {
  value: DsdStrategy;
  label: string;
  description: string;
}[] = [
  {
    value: "auto",
    label: "Automatic",
    description:
      "Native DSD when the driver reports it, otherwise convert to PCM.",
  },
  {
    value: "native",
    label: "Native DSD",
    description:
      "Send DSD in the driver's DSD format (ALSA DSD_U32_BE and similar).",
  },
  {
    value: "dop",
    label: "DSD over PCM (DoP v1.0)",
    description:
      "Pack DSD into 24-bit PCM frames under DoP markers. Only for DACs that decode DoP.",
  },
  {
    value: "pcm",
    label: "Convert to PCM",
    description:
      "Always convert DSD to PCM, which the DSP engine can then process.",
  },
];

/** DSD64, DSD128, DSD256 and DSD512 one-bit rates. */
export const DSD_RATES = [
  2_822_400, 5_644_800, 11_289_600, 22_579_200,
] as const;

/** `DSD64` for 2 822 400 Hz. */
export function dsdRateLabel(rate: number): string {
  return `DSD${Math.round(rate / 44_100)}`;
}

export interface NativeOutputSettings {
  /** Open the device exclusively (bit-perfect). */
  exclusive: boolean;
  /** Highest PCM rate to send, or `null` for whatever the device takes. */
  max_sample_rate: number | null;
  /** Highest bit depth to send, or `null`. */
  max_bit_depth: number | null;
  /** DSD playback strategy (older servers omit it: `auto`). */
  dsd_strategy?: DsdStrategy;
  /** Highest DSD rate sent as DSD; above it DSD is converted to PCM. */
  max_dsd_rate?: number | null;
}

/** One playback alternate setting a USB DAC's descriptors declare. */
export interface UsbAltset {
  interface: number | null;
  altset: number | null;
  /** Kernel format names, e.g. `S24_3LE`, `DSD_U32_BE`. */
  formats: string[];
  channels: number | null;
  rates: number[];
  bits: number | null;
}

/** What the operating system reports about a device's hardware. */
export interface NativeHardwareInfo {
  bus: string;
  /** e.g. `21b4:0082`. */
  usb_id: string | null;
  /** From the USB descriptors, e.g. `AudioQuest DragonFly Red v1.0`. */
  product: string | null;
  /** e.g. `high speed`. */
  speed: string | null;
  /** `driver` when libasound probed it; `usb_descriptors` when busy. */
  caps_source: "driver" | "usb_descriptors";
  altsets: UsbAltset[];
  /**
   * The card is answered by Parson's kernel-ABI emulator (a test bench
   * without sound hardware), not by a kernel driver and a DAC.
   */
  kernel_emulator?: boolean;
}

/** The ALSA plugin chain behind an opened PCM, from `snd_pcm_dump`. */
export interface NativePcmChain {
  /** Top to bottom, e.g. `["File PCM", "Null PCM"]`. */
  plugins: string[];
  /** No plugin in the chain can change a sample. */
  bit_exact: boolean;
  /** The plugins that convert, mix or scale. */
  converting: string[];
}

/** The driver's report of what a playing device runs at. */
export interface NativeHardwareStatus {
  /**
   * The file read, e.g. `/proc/asound/card1/pcm0p/sub0/hw_params`, or
   * `snd_pcm_dump(<pcm>)` for a libasound report.
   */
  source: string;
  /**
   * `kernel` (procfs), `library` (libasound's opened chain) or
   * `renderer` (a network player's own report, OpenHome Info).
   */
  reporter?: "kernel" | "library" | "renderer";
  /** The report came from Parson's kernel-ABI emulator, not a kernel. */
  kernel_emulator?: boolean;
  /** The plugin chain the samples pass through, for libasound reports. */
  chain?: NativePcmChain | null;
  params: {
    access: string | null;
    /** e.g. `S24_3LE`. */
    format: string;
    channels: number;
    rate: number;
    period_size: number | null;
    buffer_size: number | null;
  };
  /** A USB DAC's measured clock while streaming. */
  momentary_hz: number | null;
  /** The hardware runs exactly what the server opened. */
  matches_stream: boolean;
}

export interface FormatSupportEntry {
  value: number;
  /** The device accepts it bit-perfectly. */
  supported: boolean;
  /** The device's max settings let it through. */
  allowed: boolean;
}

export interface NativeOutputDevice {
  id: string;
  /** e.g. `Pre Box RS2 Digital`. */
  name: string;
  /** e.g. `hw:CARD=USB,DEV=0 · ...` or `USB`. */
  detail: string;
  driver: NativeOutputDriver;
  /**
   * e.g. `ALSA hw:1,0 exclusive` or `WASAPI Exclusive Mode`, following
   * the exclusive setting.
   */
  mode_label: string;
  /** How the driver addresses it: `hw:1,0`, or an ALSA PCM's name. */
  address?: string | null;
  exclusive_capable: boolean;
  is_default: boolean;
  /** Why the device could not be probed (e.g. in use by another app). */
  probe_error: string | null;
  settings: NativeOutputSettings;
  /** Every standard rate and depth; `null` when not probed. */
  formats: {
    sample_rates: FormatSupportEntry[];
    bit_depths: FormatSupportEntry[];
    /** DSD rates: `supported` natively, `allowed` by the max DSD rate. */
    dsd_rates?: FormatSupportEntry[];
    /** DSD rates whose DoP carrier the device accepts as PCM. */
    dop_rates?: number[];
  } | null;
  /** Sample containers, e.g. `24-bit packed`. */
  sample_formats: string[];
  /** `[min, max]` channels. */
  channels: [number, number] | null;
  /** Parson is playing on it now. */
  in_use: boolean;
  /**
   * The device's DSP chain, which the server runs in 64-bit float before
   * the device (older servers omit it).
   */
  dsp?: DspSettings;
  /** The chain was saved for this device rather than left at defaults. */
  dsp_customized?: boolean;
  dsp_support?: NativeDspSupport;
  /** Native DSD containers the driver accepts, e.g. `DSD_U32_BE`. */
  dsd_formats?: string[];
  /** What the kernel says about the hardware (Linux). */
  hardware?: NativeHardwareInfo | null;
  /** A network player's protocol, identity and format limits. */
  network?: NetworkOutputInfo | null;
}

/** What the server's DSP chain can do on one device. */
export interface NativeDspSupport {
  available: boolean;
  /** Why it cannot run, when it cannot. */
  reason: string | null;
  /** e.g. `Parson DSP, 64-bit float`. */
  engine: string;
  /** The device opens in stereo, which crossfeed needs. */
  crossfeed: boolean;
  /** Fixed DSP rates the device accepts as configured. */
  sample_rates: number[];
  /** Depth the chain dithers to. */
  output_bits: number | null;
}

export interface NativeOutputs {
  /** This server can open devices natively. */
  supported: boolean;
  driver: NativeOutputDriver | null;
  /** The simulated test device is enabled. */
  simulated: boolean;
  devices: NativeOutputDevice[];
  /** Network players (older servers omit it). */
  network?: NetworkOutputsView;
}

export type NativePlaybackState =
  "starting" | "playing" | "paused" | "ended" | "stopped" | "failed";

/** How a track is sent to the device. */
export interface NativeOutputPlan {
  sample_rate: number;
  sample_format: NativeSampleFormat;
  bit_depth: number;
  channels: number;
  resample: [number, number] | null;
  depth:
    | { kind: "none" }
    | { kind: "pad"; from: number; to: number }
    | { kind: "reduce"; from: number; to: number }
    | { kind: "quantize"; to: number };
  channel_change: [number, number] | null;
  exclusive: boolean;
  bit_perfect: boolean;
  /** How a DSD source is sent; absent for PCM. */
  dsd?: NativeDsdPlan;
}

export interface NativeDsdPlan {
  mode: "native" | "dop" | "pcm";
  dsd_rate: number;
  /** e.g. `DSD64`. */
  dsd_label: string;
  strategy: DsdStrategy;
  /** e.g. `DSD_U32_BE`, or the DoP carrier's container. */
  container: string | null;
  frame_rate: number | null;
  /** The PCM rate DSD is decoded to, when converted. */
  pcm_rate: number | null;
  /** Why DSD was converted although the strategy asked for DSD. */
  fallback_reason: string | null;
}

export interface NativeOutputSession {
  device_id: string;
  device_name: string;
  song_id: string;
  user_id: number;
  duration_ms: number;
  state: NativePlaybackState;
  /** What is being heard now. */
  position_ms: number;
  error: string | null;
  plan: NativeOutputPlan;
  leveling: NativeOutputLeveling | null;
  signal_path: ServerSignalPath;
  /** What the hardware runs at, from the kernel, while playing. */
  hardware?: NativeHardwareStatus | null;
}

/**
 * Saves a server output's DSP chain. The server runs it before the
 * device; a track playing there restarts at its position with it.
 */
export async function updateNativeOutputDsp(
  deviceId: string,
  settings: DspSettings,
): Promise<NativeOutputDevice> {
  return (
    await api.put<NativeOutputDevice>("/outputs/native/dsp", {
      device_id: deviceId,
      settings,
    })
  ).data;
}

/** Volume leveling the server applies (ReplayGain), in dB. */
export interface NativeOutputLeveling {
  gain_db: number;
  source: "track" | "album";
}

export type NativeOutputCommand = "pause" | "resume" | "seek" | "stop";

/** The server's audio devices, with bit-perfect format support. */
export async function getNativeOutputs(): Promise<NativeOutputs> {
  return (await api.get<NativeOutputs>("/outputs/native")).data;
}

/** Re-probes the devices (administrators). */
export async function refreshNativeOutputs(): Promise<NativeOutputs> {
  return (await api.post<NativeOutputs>("/outputs/native/admin/refresh", {}))
    .data;
}

/** Saves one device's settings (administrators). */
export async function updateNativeOutputSettings(
  deviceId: string,
  settings: NativeOutputSettings,
): Promise<NativeOutputDevice> {
  return (
    await api.put<NativeOutputDevice>("/outputs/native/admin/settings", {
      device_id: deviceId,
      settings,
    })
  ).data;
}

/** What the server is playing on its own output, or `null`. */
export async function getNativeOutputSession(): Promise<NativeOutputSession | null> {
  return (
    (await api.get<NativeOutputSession | null>("/outputs/native/session"))
      .data ?? null
  );
}

/** Plays a song on a server device, replacing what is playing there. */
export async function playOnNativeOutput(input: {
  device_id: string;
  song_id: string;
  position_ms?: number;
  /** Leveling for the server to apply; omitted when off. */
  leveling?: NativeOutputLeveling | null;
}): Promise<NativeOutputSession> {
  const body: Record<string, unknown> = {
    device_id: input.device_id,
    song_id: input.song_id,
    position_ms: Math.max(0, Math.round(input.position_ms ?? 0)),
  };
  if (input.leveling) body.leveling = input.leveling;
  return (await api.post<NativeOutputSession>("/outputs/native/session", body))
    .data;
}

/** Transport control; `stop` releases the device and returns `null`. */
export async function sendNativeOutputCommand(
  command: NativeOutputCommand,
  positionMs?: number,
): Promise<NativeOutputSession | null> {
  const body: { command: NativeOutputCommand; position_ms?: number } = {
    command,
  };
  if (positionMs !== undefined)
    body.position_ms = Math.max(0, Math.round(positionMs));
  const response = await api.post<NativeOutputSession | null>(
    "/outputs/native/session/commands",
    body,
  );
  return command === "stop" ? null : (response.data ?? null);
}

/** `44.1kHz`-style label for a rate entry. */
export function nativeFormatLabel(rate: number) {
  const khz = rate / 1000;
  return Number.isInteger(khz) ? `${khz}kHz` : `${khz.toFixed(1)}kHz`;
}

/** Adds a network player by address and returns the refreshed list. */
export async function addNetworkOutput(
  endpoint: ManualNetworkEndpoint,
): Promise<NativeOutputs> {
  return (
    await api.post<NativeOutputs>(
      "/outputs/native/admin/network",
      {
        protocol: endpoint.protocol,
        address: endpoint.address.trim(),
        name: endpoint.name?.trim() || null,
      },
      { timeout: 30_000 },
    )
  ).data;
}

/** Removes a network player added by address. */
export async function removeNetworkOutput(
  address: string,
): Promise<NativeOutputs> {
  return (
    await api.delete<NativeOutputs>("/outputs/native/admin/network", {
      data: { address },
      timeout: 30_000,
    })
  ).data;
}

/** URL of a network player's own picture (same-origin, cookie auth). */
export function nativeOutputIconUrl(deviceId: string) {
  const base = getApiBaseURL().replace(/\/+$/, "");
  return `${base}/outputs/native/icon?device_id=${encodeURIComponent(deviceId)}`;
}

/** A short identity line for a network player: maker, model, protocol. */
export function networkOutputIdentity(network: NetworkOutputInfo): string {
  const maker = network.manufacturer?.trim();
  const model = network.model?.trim();
  const name =
    maker && model && !model.toLowerCase().includes(maker.toLowerCase())
      ? `${maker} ${model}`
      : (model ?? maker ?? null);
  return [name, networkProtocolLabels[network.protocol]]
    .filter(Boolean)
    .join(" · ");
}

/* ------------------------------------------------------------------ */
/* Convolution filters                                                 */
/* ------------------------------------------------------------------ */

export interface ConvolutionFilterSummary {
  sample_rate: number;
  channels: number;
  taps: number;
  duration_ms: number;
  /** Highest gain of the filter's response: the headroom it needs. */
  peak_gain_db: number;
  gain_1khz_db: number;
  /** e.g. `32-bit float`. */
  format: string;
}

export interface ConvolutionFilter {
  id: string;
  name: string;
  file_name: string | null;
  added_at: string;
  summary: ConvolutionFilterSummary;
  /** Magnitude response of the first channel, `[Hz, dB]`. */
  response: [number, number][];
}

/** The convolution filters stored on the server. */
export async function getConvolutionFilters(): Promise<ConvolutionFilter[]> {
  return (await api.get<ConvolutionFilter[]>("/outputs/native/convolution"))
    .data;
}

/** Uploads a WAV impulse response (administrators). */
export async function uploadConvolutionFilter(
  file: Blob,
  options: { name?: string; fileName?: string } = {},
): Promise<ConvolutionFilter> {
  const params: Record<string, string> = {};
  if (options.name?.trim()) params.name = options.name.trim();
  if (options.fileName) params.file_name = options.fileName;
  return (
    await api.post<ConvolutionFilter>(
      "/outputs/native/admin/convolution",
      file,
      {
        params,
        headers: { "Content-Type": "audio/wav" },
        timeout: 120_000,
      },
    )
  ).data;
}

export async function deleteConvolutionFilter(id: string): Promise<void> {
  await api.delete(
    `/outputs/native/admin/convolution/${encodeURIComponent(id)}`,
  );
}

/** The filter's WAV file, for a browser's own `ConvolverNode`. */
export async function getConvolutionFilterFile(
  id: string,
): Promise<ArrayBuffer> {
  const blob = (
    await api.get<Blob>(
      `/outputs/native/convolution/${encodeURIComponent(id)}/file`,
      { responseType: "blob", timeout: 60_000 },
    )
  ).data;
  return blob.arrayBuffer();
}

/** The server's own explanation of a failed outputs request. */
export function outputErrorMessage(error: unknown, fallback: string): string {
  if (isApiError(error)) {
    const data = error.response?.data as { message?: unknown } | undefined;
    if (typeof data?.message === "string" && data.message) return data.message;
  }
  return error instanceof Error && error.message ? error.message : fallback;
}
