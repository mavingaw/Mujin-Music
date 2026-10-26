"use client";

import {
  defaultDspSettings,
  normalizeDspSettings,
  updateNativeOutputDsp,
  type DspSettings,
  type NativeOutputDevice,
  type NativeOutputs,
} from "@parson/music-sdk";
import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { nativeOutputsKey } from "./use-native-outputs";

/** Saves are batched while a slider moves: each save restarts the track. */
export const SERVER_DSP_SAVE_DELAY_MS = 350;

/**
 * The chain a server output should start with: what the server saved
 * for it, or (for a device the server has no chain for yet) what this
 * browser kept for it locally, so settings made before the server ran
 * the chain itself are not lost.
 */
export function initialServerDsp(
  device: Pick<NativeOutputDevice, "dsp" | "dsp_customized"> | null,
  local: DspSettings,
): { settings: DspSettings; push: boolean } {
  if (device?.dsp && device.dsp_customized)
    return { settings: normalizeDspSettings(device.dsp), push: false };
  const localChanged =
    JSON.stringify(normalizeDspSettings(local)) !==
    JSON.stringify(defaultDspSettings());
  if (localChanged) return { settings: local, push: Boolean(device?.dsp) };
  return {
    settings: device?.dsp ? normalizeDspSettings(device.dsp) : local,
    push: false,
  };
}

/**
 * DSP settings for a server output. The server keeps the chain (it runs
 * it, and every client controlling that output sees the same one); this
 * browser mirrors it locally. Changes show at once and are saved shortly
 * after, which restarts a playing track at its position with the chain.
 */
export function useServerDsp(
  deviceId: string | null,
  device: NativeOutputDevice | null,
  local: DspSettings,
  setLocal: (settings: DspSettings) => void,
) {
  const queryClient = useQueryClient();
  const [settings, setSettingsState] = useState<DspSettings>(local);
  const [error, setError] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const seeded = useRef<string | null>(null);

  const save = useCallback(
    (id: string, next: DspSettings) => {
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        timer.current = null;
        updateNativeOutputDsp(id, next)
          .then((saved) => {
            setError(null);
            queryClient.setQueryData<NativeOutputs>(nativeOutputsKey, (old) =>
              old
                ? {
                    ...old,
                    devices: old.devices.map((item) =>
                      item.id === saved.id ? saved : item,
                    ),
                  }
                : old,
            );
            void queryClient.invalidateQueries({ queryKey: ["signal-path"] });
          })
          .catch((cause: unknown) =>
            setError(
              cause instanceof Error
                ? cause.message
                : "The server did not save the DSP settings.",
            ),
          );
      }, SERVER_DSP_SAVE_DELAY_MS);
    },
    [queryClient],
  );

  // Adopt the server's chain when the device (or its saved chain) changes.
  useEffect(() => {
    if (!deviceId || !device) return;
    const key = `${deviceId}:${JSON.stringify(device.dsp ?? null)}`;
    if (seeded.current === key) return;
    const first = !seeded.current?.startsWith(`${deviceId}:`);
    seeded.current = key;
    if (!first && timer.current) return;
    const initial = initialServerDsp(device, local);
    setSettingsState(initial.settings);
    if (initial.push) save(deviceId, initial.settings);
  }, [device, deviceId, local, save]);

  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );

  const setSettings = useCallback(
    (next: DspSettings) => {
      const normalized = normalizeDspSettings(next);
      setSettingsState(normalized);
      setLocal(normalized);
      if (deviceId) save(deviceId, normalized);
    },
    [deviceId, save, setLocal],
  );

  return { settings, setSettings, error };
}
