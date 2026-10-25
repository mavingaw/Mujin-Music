import api from "../core/http";
import type { ServerSignalPath } from "../player/signal-path";

/** How the server drives a device. */
export type NativeOutputDriver = "alsa" | "wasapi" | "core_audio" | "simulated";

export type NativeSampleFormat = "s16" | "s24_packed" | "s24_in32" | "s32";

export interface NativeOutputSettings {
  /** Open the device exclusively (bit-perfect). */
  exclusive: boolean;
  /** Highest PCM rate to send, or `null` for whatever the device takes. */
  max_sample_rate: number | null;
  /** Highest bit depth to send, or `null`. */
  max_bit_depth: number | null;
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
  /** e.g. `WASAPI Exclusive Mode`, following the exclusive setting. */
  mode_label: string;
  exclusive_capable: boolean;
  is_default: boolean;
  /** Why the device could not be probed (e.g. in use by another app). */
  probe_error: string | null;
  settings: NativeOutputSettings;
  /** Every standard rate and depth; `null` when not probed. */
  formats: {
    sample_rates: FormatSupportEntry[];
    bit_depths: FormatSupportEntry[];
  } | null;
  /** Sample containers, e.g. `24-bit packed`. */
  sample_formats: string[];
  /** `[min, max]` channels. */
  channels: [number, number] | null;
  /** Parson is playing on it now. */
  in_use: boolean;
}

export interface NativeOutputs {
  /** This server can open devices natively. */
  supported: boolean;
  driver: NativeOutputDriver | null;
  /** The simulated test device is enabled. */
  simulated: boolean;
  devices: NativeOutputDevice[];
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
