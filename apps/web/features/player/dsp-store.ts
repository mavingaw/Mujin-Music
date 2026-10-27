"use client";

import {
  defaultDspSettings,
  normalizeDspSettings,
  type DspSettings,
} from "@parson/music-sdk";
import { useCallback, useEffect, useState, useSyncExternalStore } from "react";

const DSP_KEY = "parson.dsp-chains";
const OUTPUT_KEY = "parson.output-device";
const listeners = new Set<() => void>();

/** DSP settings are kept per output device, like Roon's per-zone MUSE. */
export function outputKey(sinkId: string | null | undefined) {
  return sinkId && sinkId !== "default" ? sinkId : "default";
}

export type DspChains = Record<string, DspSettings>;

export function parseDspChains(raw: string | null | undefined): DspChains {
  if (!raw) return {};
  try {
    const value = JSON.parse(raw) as unknown;
    if (!value || typeof value !== "object" || Array.isArray(value)) return {};
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>)
        .slice(0, 32)
        .map(([key, settings]) => [key, normalizeDspSettings(settings)]),
    );
  } catch {
    return {};
  }
}

export function dspForOutput(chains: DspChains, sinkId: string | null) {
  return chains[outputKey(sinkId)] ?? defaultDspSettings();
}

function read(key: string) {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {}
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  const onStorage = (event: StorageEvent) => {
    if (event.key === DSP_KEY || event.key === OUTPUT_KEY) listener();
  };
  window.addEventListener("storage", onStorage);
  return () => {
    listeners.delete(listener);
    window.removeEventListener("storage", onStorage);
  };
}

/** The chosen output device id, `""` for the system default. */
export function useOutputDeviceChoice() {
  const id = useSyncExternalStore(
    subscribe,
    () => read(OUTPUT_KEY) ?? "",
    () => "",
  );
  const setId = useCallback((next: string) => write(OUTPUT_KEY, next), []);
  return [id, setId] as const;
}

/** DSP settings for one output, remembered per browser. */
export function useDspSettings(sinkId: string) {
  const raw = useSyncExternalStore(
    subscribe,
    () => read(DSP_KEY),
    () => null,
  );
  const [settings, setSettingsState] = useState<DspSettings>(() =>
    dspForOutput(parseDspChains(raw), sinkId),
  );
  useEffect(() => {
    setSettingsState(dspForOutput(parseDspChains(raw), sinkId));
  }, [raw, sinkId]);
  const setSettings = useCallback(
    (next: DspSettings) => {
      setSettingsState(next);
      const chains = parseDspChains(read(DSP_KEY));
      chains[outputKey(sinkId)] = normalizeDspSettings(next);
      write(DSP_KEY, JSON.stringify(chains));
    },
    [sinkId],
  );
  return [settings, setSettings] as const;
}

export type OutputDevice = { id: string; label: string };

/** Audio outputs from `enumerateDevices()`, without the duplicates Chrome adds. */
export function audioOutputs(
  devices: readonly Pick<MediaDeviceInfo, "deviceId" | "kind" | "label">[],
): OutputDevice[] {
  const seen = new Set<string>();
  const outputs: OutputDevice[] = [];
  for (const device of devices) {
    if (device.kind !== "audiooutput") continue;
    if (device.deviceId === "communications") continue;
    if (seen.has(device.deviceId)) continue;
    seen.add(device.deviceId);
    outputs.push({ id: device.deviceId, label: device.label });
  }
  return outputs;
}

function stripDefault(label: string) {
  return label.replace(/^Default\s*[-–]\s*/i, "");
}

/**
 * Name of the device audio goes to: the chosen one, or what the system
 * default currently points at. `null` when the browser hides labels.
 */
export function outputDeviceLabel(
  devices: readonly OutputDevice[],
  sinkId: string,
): string | null {
  if (sinkId && sinkId !== "default") {
    const chosen = devices.find((device) => device.id === sinkId);
    if (chosen?.label) return chosen.label;
  }
  const fallback = devices.find((device) => device.id === "default");
  if (fallback?.label) return stripDefault(fallback.label);
  const first = devices.find((device) => device.label);
  return devices.length === 1 && first ? first.label : null;
}

export function supportsOutputSelection() {
  return (
    typeof window !== "undefined" &&
    typeof HTMLMediaElement !== "undefined" &&
    "setSinkId" in HTMLMediaElement.prototype
  );
}

/** Lists audio outputs and keeps the list fresh as devices come and go. */
export function useAudioOutputs() {
  const [devices, setDevices] = useState<OutputDevice[]>([]);
  const refresh = useCallback(async () => {
    if (!navigator.mediaDevices?.enumerateDevices) return;
    try {
      setDevices(audioOutputs(await navigator.mediaDevices.enumerateDevices()));
    } catch {}
  }, []);
  useEffect(() => {
    void refresh();
    navigator.mediaDevices?.addEventListener?.("devicechange", refresh);
    return () =>
      navigator.mediaDevices?.removeEventListener?.("devicechange", refresh);
  }, [refresh]);
  /**
   * Browsers hide device names until the site has a media permission.
   * Firefox asks with `selectAudioOutput`; Chromium reveals output names
   * once microphone access is granted, and the stream is stopped at once.
   */
  const requestLabels = useCallback(async () => {
    const media = navigator.mediaDevices as MediaDevices & {
      selectAudioOutput?: () => Promise<MediaDeviceInfo>;
    };
    try {
      if (media.selectAudioOutput) {
        await media.selectAudioOutput();
      } else if (media.getUserMedia) {
        const stream = await media.getUserMedia({ audio: true });
        for (const track of stream.getTracks()) track.stop();
      }
    } catch {}
    await refresh();
  }, [refresh]);
  const labelsHidden =
    devices.length > 0 && devices.every((device) => !device.label);
  return { devices, labelsHidden, refresh, requestLabels };
}
