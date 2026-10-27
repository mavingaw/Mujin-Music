"use client";

import {
  deleteConvolutionFilter,
  formatDecibels,
  formatSampleRate,
  getConvolutionFilters,
  outputErrorMessage,
  uploadConvolutionFilter,
  type ConvolutionFilter,
  type DspSettings,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, Trash2, Upload } from "lucide-react";
import { useMemo, useRef } from "react";
import { useSession } from "@/features/account/session-provider";
import { cn } from "@/lib/cn";

export const convolutionFiltersKey = ["convolution-filters"] as const;

export function useConvolutionFilters(enabled = true) {
  const { session } = useSession();
  return useQuery<ConvolutionFilter[]>({
    queryKey: convolutionFiltersKey,
    queryFn: getConvolutionFilters,
    enabled: enabled && Boolean(session?.sub),
    staleTime: 60_000,
    retry: 1,
  });
}

/** `65,536 taps · 48kHz · stereo · 1365 ms · 32-bit float`. */
export function describeFilter(filter: ConvolutionFilter) {
  const summary = filter.summary;
  return [
    `${summary.taps.toLocaleString("en-US")} taps`,
    formatSampleRate(summary.sample_rate),
    summary.channels === 1
      ? "mono (all channels)"
      : summary.channels === 2
        ? "stereo"
        : `${summary.channels} channels`,
    `${Math.round(summary.duration_ms)} ms`,
    summary.format,
  ].join(" · ");
}

/** Headroom a filter needs: its response's peak gain, when above 0 dB. */
export function filterHeadroomDb(filter: ConvolutionFilter) {
  const peak = filter.summary.peak_gain_db;
  return peak > 0.05 ? -Math.ceil(peak * 2) / 2 : 0;
}

function FilterCurve({ filter }: { filter: ConvolutionFilter }) {
  const { points, range } = useMemo(() => {
    const values = filter.response.map(([, db]) => db);
    const top = Math.max(6, ...values.map((db) => Math.ceil(db / 6) * 6));
    const bottom = Math.min(-12, ...values.map((db) => Math.floor(db / 6) * 6));
    const span = Math.max(12, top - bottom);
    const low = Math.log10(20);
    const high = Math.log10(20_000);
    return {
      range: { top, bottom, span },
      points: filter.response.map(([frequency, db]) => {
        const x = ((Math.log10(frequency) - low) / (high - low)) * 100;
        const y = ((top - Math.max(bottom, Math.min(top, db))) / span) * 100;
        return `${x.toFixed(2)},${y.toFixed(2)}`;
      }),
    };
  }, [filter]);
  const zero = (range.top / range.span) * 100;
  return (
    <svg
      aria-label={`${filter.name} frequency response`}
      className="h-20 w-full rounded-lg bg-black/40"
      data-filter-curve
      preserveAspectRatio="none"
      role="img"
      viewBox="0 0 100 100"
    >
      <line
        stroke="#52525b"
        strokeWidth={0.4}
        vectorEffect="non-scaling-stroke"
        x1={0}
        x2={100}
        y1={zero}
        y2={zero}
      />
      <polyline
        fill="none"
        points={points.join(" ")}
        stroke="#8FC3E0"
        strokeWidth={1.6}
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}

/**
 * The convolution section of the DSP engine: choose a stored FIR filter
 * (room correction), see its response and the headroom it needs, and,
 * as an administrator, upload or delete filters.
 */
export function ConvolutionControls({
  settings,
  onChange,
  serverRun,
}: {
  settings: DspSettings;
  onChange: (settings: DspSettings) => void;
  serverRun: boolean;
}) {
  const { session } = useSession();
  const isAdmin = session?.role === "admin";
  const queryClient = useQueryClient();
  const filters = useConvolutionFilters();
  const fileRef = useRef<HTMLInputElement>(null);
  const convolution = settings.convolution ?? {
    enabled: false,
    filterId: null,
  };
  const selected =
    filters.data?.find((filter) => filter.id === convolution.filterId) ?? null;
  const choose = (filterId: string | null, enabled = convolution.enabled) =>
    onChange({ ...settings, convolution: { enabled, filterId } });
  const upload = useMutation({
    mutationFn: (file: File) =>
      uploadConvolutionFilter(file, { fileName: file.name }),
    onSuccess: (filter) => {
      queryClient.setQueryData<ConvolutionFilter[]>(
        convolutionFiltersKey,
        (current) => [...(current ?? []), filter],
      );
      choose(filter.id, true);
    },
  });
  const remove = useMutation({
    mutationFn: (id: string) => deleteConvolutionFilter(id),
    onSuccess: (_, id) => {
      queryClient.setQueryData<ConvolutionFilter[]>(
        convolutionFiltersKey,
        (current) => (current ?? []).filter((filter) => filter.id !== id),
      );
      if (convolution.filterId === id) choose(null, false);
    },
  });
  const headroom = selected ? filterHeadroomDb(selected) : 0;
  const headroomShort =
    headroom < 0 &&
    (!settings.headroom.enabled || settings.headroom.gainDb > headroom);

  return (
    <div className="space-y-2" data-convolution>
      <div className="flex items-center gap-2">
        <select
          aria-label="Convolution filter"
          className="h-8 min-w-0 flex-1 rounded-md border border-white/10 bg-zinc-900 px-2 text-xs text-zinc-100"
          onChange={(event) => choose(event.target.value || null)}
          value={convolution.filterId ?? ""}
        >
          <option value="">
            {filters.isLoading
              ? "Loading filters…"
              : filters.data?.length
                ? "Choose a filter"
                : "No filters on the server yet"}
          </option>
          {filters.data?.map((filter) => (
            <option key={filter.id} value={filter.id}>
              {filter.name}
            </option>
          ))}
          {convolution.filterId && !selected && !filters.isLoading ? (
            <option value={convolution.filterId}>
              Missing filter (deleted)
            </option>
          ) : null}
        </select>
        {isAdmin ? (
          <>
            <input
              accept=".wav,audio/wav,audio/x-wav"
              className="hidden"
              onChange={(event) => {
                const file = event.target.files?.[0];
                if (file) upload.mutate(file);
                event.target.value = "";
              }}
              ref={fileRef}
              type="file"
            />
            <button
              className="inline-flex h-8 items-center gap-1.5 rounded-md border border-white/10 px-2.5 text-xs text-zinc-200 hover:bg-white/6 disabled:opacity-50"
              disabled={upload.isPending}
              onClick={() => fileRef.current?.click()}
              type="button"
            >
              <Upload className="h-3.5 w-3.5" />
              {upload.isPending ? "Uploading" : "Upload WAV"}
            </button>
          </>
        ) : null}
      </div>
      {selected ? (
        <div
          className="space-y-1.5 rounded-lg border border-white/8 px-3 py-2"
          data-convolution-filter={selected.id}
        >
          <div className="flex items-start gap-2">
            <p className="min-w-0 flex-1 text-[11px] leading-4 text-zinc-400">
              <span className="font-medium text-zinc-200">{selected.name}</span>
              {" · "}
              {describeFilter(selected)}. Peaks at{" "}
              {formatDecibels(selected.summary.peak_gain_db)}.
            </p>
            {isAdmin ? (
              <button
                aria-label={`Delete ${selected.name}`}
                className="flex h-6 w-6 items-center justify-center rounded text-zinc-500 hover:bg-white/8 hover:text-white"
                disabled={remove.isPending}
                onClick={() => remove.mutate(selected.id)}
                type="button"
              >
                <Trash2 className="h-3.5 w-3.5" />
              </button>
            ) : null}
          </div>
          <FilterCurve filter={selected} />
          <div className="flex justify-between text-[10px] text-zinc-600">
            <span>20 Hz</span>
            <span>1 kHz</span>
            <span>20 kHz</span>
          </div>
          {headroomShort ? (
            <p className="flex flex-wrap items-center gap-2 text-[11px] text-amber-300/90">
              <AlertTriangle className="h-3 w-3" />
              This filter boosts up to{" "}
              {formatDecibels(selected.summary.peak_gain_db)}.
              <button
                className="rounded-full border border-amber-300/30 px-2 py-0.5 hover:bg-amber-300/10"
                onClick={() =>
                  onChange({
                    ...settings,
                    headroom: { enabled: true, gainDb: headroom },
                  })
                }
                type="button"
              >
                Use {formatDecibels(headroom)} headroom
              </button>
            </p>
          ) : null}
          <p className="text-[10px] leading-4 text-zinc-500">
            {serverRun
              ? `The server convolves in 64-bit float with partitioned FFT convolution, no added delay${selected.summary.sample_rate ? `, converting the filter from ${formatSampleRate(selected.summary.sample_rate)} when a track plays at another rate` : ""}.`
              : "This browser convolves with its Web Audio ConvolverNode in 32-bit float, unnormalised, after fetching the filter from the server."}
          </p>
        </div>
      ) : null}
      {upload.error || remove.error ? (
        <p className={cn("text-[11px] text-red-300")}>
          {outputErrorMessage(
            upload.error ?? remove.error,
            "The filter could not be saved.",
          )}
        </p>
      ) : null}
      {!isAdmin && !filters.data?.length ? (
        <p className="text-[11px] text-zinc-500">
          An administrator can upload WAV impulse responses (REW, Dirac,
          Acourate and similar exports).
        </p>
      ) : null}
    </div>
  );
}
