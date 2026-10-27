/**
 * What the server's outputs are made of: the physical endpoints its zones
 * play through (format support, hardware, network identity), the players
 * added by address, and the convolution filters its DSP chain runs. The
 * outputs themselves are zones (`resources/zones`): their device setup and
 * DSP chain are zone settings.
 */
import api, { getApiBaseURL, isApiError } from "../core/http";

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
  assumed: "Assumed by Mujin Music (the player does not say)",
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
  /** The player's own picture (fetch it through `zoneIconUrl`). */
  icon_url: string | null;
  kind: NetworkEndpointKind;
  caps_source: NetworkCapsSource;
  /** e.g. `ALAC 44.1kHz 16bit over RTP (AirPlay 1)`. */
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

/** Players added by address, and whether the server searches for more. */
export interface NetworkPlayers {
  /** SSDP and mDNS discovery is on. */
  discovery: boolean;
  manual: ManualNetworkEndpoint[];
}

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
export interface OutputHardwareInfo {
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
export interface OutputPcmChain {
  /** Top to bottom, e.g. `["File PCM", "Null PCM"]`. */
  plugins: string[];
  /** No plugin in the chain can change a sample. */
  bit_exact: boolean;
  /** The plugins that convert, mix or scale. */
  converting: string[];
}

export interface FormatSupportEntry {
  value: number;
  /** The device accepts it bit-perfectly. */
  supported: boolean;
  /** The device's max settings let it through. */
  allowed: boolean;
}

/** Every standard rate, depth and DSD rate an endpoint is asked about. */
export interface OutputFormatSupport {
  sample_rates: FormatSupportEntry[];
  bit_depths: FormatSupportEntry[];
  /** DSD rates: `supported` natively, `allowed` by the max DSD rate. */
  dsd_rates?: FormatSupportEntry[];
  /** DSD rates whose DoP carrier the device accepts as PCM. */
  dop_rates?: number[];
}

/** What the server's DSP chain can do for one endpoint. */
export interface OutputDspSupport {
  available: boolean;
  /** Why it cannot run, when it cannot. */
  reason: string | null;
  /** e.g. `Parson DSP, 64-bit float`. */
  engine: string;
  /** The output plays stereo, which crossfeed needs. */
  crossfeed: boolean;
  /** Fixed DSP rates the output accepts as configured. */
  sample_rates: number[];
  /** Depth the chain dithers to. */
  output_bits: number | null;
}

/** The UPnP, OpenHome and AirPlay players added by address. */
export async function getNetworkPlayers(): Promise<NetworkPlayers> {
  return (await api.get<NetworkPlayers>("/zones/outputs/network")).data;
}

/** Adds (or renames) a network player by address (administrators). */
export async function addNetworkPlayer(
  endpoint: ManualNetworkEndpoint,
): Promise<NetworkPlayers> {
  return (
    await api.post<NetworkPlayers>(
      "/zones/outputs/network",
      {
        protocol: endpoint.protocol,
        address: endpoint.address.trim(),
        name: endpoint.name?.trim() || null,
      },
      { timeout: 30_000 },
    )
  ).data;
}

/** Removes a network player added by address (administrators). */
export async function removeNetworkPlayer(
  address: string,
): Promise<NetworkPlayers> {
  return (
    await api.delete<NetworkPlayers>("/zones/outputs/network", {
      data: { address },
      timeout: 30_000,
    })
  ).data;
}

/** URL of a zone's own picture, e.g. a UPnP player's icon (cookie auth). */
export function zoneIconUrl(zoneId: string) {
  const base = getApiBaseURL().replace(/\/+$/, "");
  return `${base}/zones/${encodeURIComponent(zoneId)}/icon`;
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
  return (await api.get<ConvolutionFilter[]>("/outputs/convolution")).data;
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
    await api.post<ConvolutionFilter>("/outputs/admin/convolution", file, {
      params,
      headers: { "Content-Type": "audio/wav" },
      timeout: 120_000,
    })
  ).data;
}

export async function deleteConvolutionFilter(id: string): Promise<void> {
  await api.delete(`/outputs/admin/convolution/${encodeURIComponent(id)}`);
}

/** The filter's WAV file, for a browser's own `ConvolverNode`. */
export async function getConvolutionFilterFile(
  id: string,
): Promise<ArrayBuffer> {
  const blob = (
    await api.get<Blob>(`/outputs/convolution/${encodeURIComponent(id)}/file`, {
      responseType: "blob",
      timeout: 60_000,
    })
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
