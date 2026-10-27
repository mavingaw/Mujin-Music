"use client";

import {
  defaultDspSettings,
  normalizeDspSettings,
  updateZoneSettings,
  type DspSettings,
  type OutputDspSupport,
  type Zone,
} from "@parson/music-sdk";
import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { errorText } from "./zone-settings-errors";

/** Saves are batched while a slider moves: each save replans the stream. */
export const ZONE_DSP_SAVE_DELAY_MS = 350;

/** This browser's chains were copied to its zone (one key per zone). */
const MIGRATED_KEY = "parson.dsp-migrated";

function isDefault(settings: DspSettings) {
  return (
    JSON.stringify(normalizeDspSettings(settings)) ===
    JSON.stringify(defaultDspSettings())
  );
}

/**
 * The chain a zone should start with: what the server keeps for it, or
 * (for a zone that has none yet) the chain this browser kept before zones
 * carried their own, so settings made then are not lost.
 */
export function initialZoneDsp(
  zone: DspSettings | null | undefined,
  local: DspSettings,
): { settings: DspSettings; push: boolean } {
  const saved = zone ? normalizeDspSettings(zone) : null;
  if (saved && !isDefault(saved)) return { settings: saved, push: false };
  if (!isDefault(local))
    return { settings: normalizeDspSettings(local), push: Boolean(zone) };
  return { settings: saved ?? normalizeDspSettings(local), push: false };
}

/**
 * What the server's DSP engine can do for a zone: its output's own
 * support, or (for a stream the zone fetches, which the server renders at
 * the track's rate) the engine without fixed rates.
 */
export function zoneDspSupport(zone: Zone): OutputDspSupport | null {
  const endpoint = zone.endpoints?.find((item) => item.dsp_support);
  if (endpoint?.dsp_support) return endpoint.dsp_support;
  if (!zone.settings?.supports.dsp) return null;
  return {
    available: true,
    reason: null,
    engine: "Mujin Music DSP, 64-bit float",
    crossfeed: true,
    sample_rates: [],
    output_bits: null,
  };
}

function readMigrated(): string[] {
  try {
    const value = JSON.parse(
      window.localStorage.getItem(MIGRATED_KEY) ?? "[]",
    ) as unknown;
    return Array.isArray(value)
      ? value.filter((item): item is string => typeof item === "string")
      : [];
  } catch {
    return [];
  }
}

function markMigrated(zoneId: string) {
  try {
    const ids = readMigrated().filter((id) => id !== zoneId);
    window.localStorage.setItem(
      MIGRATED_KEY,
      JSON.stringify([...ids, zoneId].slice(-16)),
    );
  } catch {}
}

/**
 * A zone's DSP chain. The server keeps it (it runs it for server-played
 * zones, and every client controlling the zone sees the same one). Changes
 * show at once and are saved shortly after. `migrateFrom` is the chain
 * this browser kept for its own output, copied once into a zone that has
 * none.
 */
export function useZoneDsp(
  zone: Zone | null,
  migrateFrom: DspSettings | null = null,
) {
  const queryClient = useQueryClient();
  const zoneId = zone?.id ?? null;
  const saved = zone?.settings?.dsp ?? null;
  const savedKey = saved ? JSON.stringify(normalizeDspSettings(saved)) : "";
  const [pending, setPending] = useState<{
    zoneId: string;
    settings: DspSettings;
  } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const save = useCallback(
    (id: string, next: DspSettings) => {
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        timer.current = null;
        updateZoneSettings(id, { dsp: next })
          .then(() => {
            setError(null);
            void queryClient.invalidateQueries({ queryKey: ["zones"] });
            void queryClient.invalidateQueries({ queryKey: ["signal-path"] });
          })
          .catch((cause: unknown) => {
            setPending(null);
            setError(
              errorText(cause, "The server did not save the DSP chain."),
            );
          });
      }, ZONE_DSP_SAVE_DELAY_MS);
    },
    [queryClient],
  );

  // A saved chain that matches the pending one ends the optimistic state.
  useEffect(() => {
    if (
      pending &&
      pending.zoneId === zoneId &&
      !timer.current &&
      JSON.stringify(pending.settings) === savedKey
    )
      setPending(null);
  }, [pending, savedKey, zoneId]);

  // Copies this browser's old chain into its zone once.
  const migrateKey = migrateFrom ? JSON.stringify(migrateFrom) : "";
  useEffect(() => {
    if (!zoneId || !migrateKey || !zone?.settings?.supports.dsp) return;
    if (readMigrated().includes(zoneId)) return;
    markMigrated(zoneId);
    const initial = initialZoneDsp(
      saved,
      JSON.parse(migrateKey) as DspSettings,
    );
    if (!initial.push) return;
    setPending({ zoneId, settings: initial.settings });
    save(zoneId, initial.settings);
    // `saved` is read once, when the zone first appears.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [migrateKey, save, zone?.settings?.supports.dsp, zoneId]);

  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );

  const settings = useMemo(
    () =>
      pending && pending.zoneId === zoneId
        ? pending.settings
        : savedKey
          ? (JSON.parse(savedKey) as DspSettings)
          : defaultDspSettings(),
    [pending, savedKey, zoneId],
  );

  const setSettings = useCallback(
    (next: DspSettings) => {
      if (!zoneId) return;
      const normalized = normalizeDspSettings(next);
      setPending({ zoneId, settings: normalized });
      save(zoneId, normalized);
    },
    [save, zoneId],
  );

  return useMemo(
    () => ({ settings, setSettings, error }),
    [error, setSettings, settings],
  );
}
