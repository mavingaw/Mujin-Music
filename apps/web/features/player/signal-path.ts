"use client";

import {
  buildSignalPath,
  getSongSignalPath,
  type PlayerAudioPreset,
  type AudioProperties,
  type ClientSignalLeg,
  type ServerSignalPath,
  type SignalPath,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { hasDesktopBridge } from "@/lib/desktop/bridge";
import type { PlayerSignalState } from "./player-api";

export type WebSignalInput = {
  casting: boolean;
  castDeviceName?: string | null;
  muted: boolean;
  preset: Pick<
    PlayerAudioPreset,
    "label" | "rate" | "wet" | "preservePitch" | "id"
  > | null;
  signal: PlayerSignalState;
  volume: number;
  desktop: boolean;
};

/** This browser's leg of the signal path. */
export function webClientLeg(input: WebSignalInput): ClientSignalLeg {
  const preset = input.preset;
  const processed = preset && preset.id !== "original";
  return {
    platform: input.desktop ? "desktop" : "web",
    engine: input.signal.engine,
    contextSampleRate: input.signal.contextSampleRate,
    playbackRate: preset?.rate ?? 1,
    preservePitch: preset?.preservePitch ?? true,
    reverbWet: preset?.wet ?? 0,
    presetLabel: processed ? preset.label : null,
    volume: input.volume,
    muted: input.muted,
    leveling: input.signal.leveling,
    levelingUnavailable: input.signal.levelingUnavailable,
    dsp: input.signal.dsp,
    convolution: input.signal.convolution ?? null,
    deviceSampleRate: input.signal.deviceSampleRate,
    output: input.casting
      ? { kind: "cast", name: input.castDeviceName }
      : input.signal.native
        ? { kind: "native", name: input.signal.native.name }
        : {
            kind: "system",
            name: input.signal.outputName,
            chosen: input.signal.outputChosen,
          },
  };
}

export function signalPathQueryKey(
  songId: string,
  bitrate: number,
  target: "direct" | "cast" | "native",
  device: string | null = null,
) {
  return ["signal-path", songId, bitrate, target, device] as const;
}

/** Server leg (fetched) plus this client's leg for the playing track. */
export function useSignalPath({
  audio,
  songId,
  ...input
}: Omit<WebSignalInput, "desktop"> & {
  audio?: AudioProperties | null;
  songId: string;
}): { path: SignalPath | null; loading: boolean } {
  const native = input.casting ? null : input.signal.native;
  const bitrate = input.casting || native ? 0 : input.signal.streamBitrate;
  const target = input.casting ? "cast" : native ? "native" : "direct";
  const device = native?.deviceId ?? null;
  const server = useQuery<ServerSignalPath>({
    queryKey: signalPathQueryKey(songId, bitrate, target, device),
    queryFn: () => getSongSignalPath(songId, { bitrate, target, device }),
    enabled: Boolean(songId),
    retry: 1,
    staleTime: 60_000,
  });
  const desktop = typeof window !== "undefined" && hasDesktopBridge();
  const path = useMemo(() => {
    if (!songId) return null;
    // The playing session's own path is authoritative for a server output.
    return buildSignalPath(
      native?.signalPath ?? server.data ?? null,
      webClientLeg({ ...input, desktop }),
      audio,
    );
  }, [
    audio,
    desktop,
    input.casting,
    input.castDeviceName,
    input.muted,
    input.preset,
    input.signal,
    input.volume,
    server.data,
    songId,
  ]);
  return { path, loading: server.isLoading };
}
