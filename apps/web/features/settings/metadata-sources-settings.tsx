"use client";

import {
  ALBUM_FIELDS,
  ALBUM_FIELD_LABELS,
  getLibrarySources,
  updateLibrarySources,
} from "@parson/music-sdk";
import type { AlbumField, FactSource } from "@parson/music-sdk/types";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Globe2, SlidersHorizontal, Tag } from "lucide-react";
import { toast } from "sonner";

const NOTES: Partial<Record<AlbumField, string>> = {
  original_date: "The year album pages show",
  release_date: "Online only from the exact edition",
  cover: "Online uses the Cover Art Archive",
  performers: "Online adds MusicBrainz to your tags",
  credits: "Composers, producers, engineers…",
};

/**
 * Library-wide "Metadata preference": per field, file tags or the matched
 * MusicBrainz release. Albums can choose differently and freeze values.
 */
export default function MetadataSourcesSettings() {
  const queryClient = useQueryClient();
  const sources = useQuery({
    queryKey: ["metadata-sources"],
    queryFn: getLibrarySources,
  });
  const update = useMutation({
    mutationFn: (changes: Partial<Record<AlbumField, FactSource>>) =>
      updateLibrarySources(changes),
    onSuccess: (data) => {
      queryClient.setQueryData(["metadata-sources"], data);
      void queryClient.invalidateQueries({ queryKey: ["album-about"] });
      void queryClient.invalidateQueries({ queryKey: ["albums"] });
    },
    onError: () => toast("Could not change the metadata preference."),
  });
  const data = sources.data;
  const allTags = data && ALBUM_FIELDS.every((field) => data[field] === "tags");
  const allOnline =
    data && ALBUM_FIELDS.every((field) => data[field] === "online");

  return (
    <section
      aria-labelledby="metadata-sources-title"
      className="border-t border-white/[0.08] pt-7"
    >
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <h2
            className="flex items-center gap-2 text-base font-semibold text-white"
            id="metadata-sources-title"
          >
            <SlidersHorizontal aria-hidden className="h-4 w-4 text-zinc-400" />
            Metadata preference
          </h2>
          <p className="mt-1 max-w-xl text-sm leading-6 text-zinc-500">
            Where each album fact comes from by default. When the chosen source
            has nothing the other fills in. Albums can choose differently under
            Edit album → Metadata preference, and values typed there are frozen
            against every lookup.
          </p>
        </div>
        <div className="flex gap-2">
          {(
            [
              ["tags", "All from tags", allTags],
              ["online", "All online", allOnline],
            ] as const
          ).map(([source, label, active]) => (
            <button
              className={`rounded-full border px-3 py-1.5 text-xs font-semibold transition-colors disabled:opacity-50 ${
                active
                  ? "border-white bg-white text-black"
                  : "border-white/12 text-zinc-300 hover:border-white/25 hover:text-white"
              }`}
              disabled={!data || update.isPending}
              key={source}
              onClick={() =>
                update.mutate(
                  Object.fromEntries(
                    ALBUM_FIELDS.map((field) => [field, source]),
                  ) as Record<AlbumField, FactSource>,
                )
              }
              type="button"
            >
              {label}
            </button>
          ))}
        </div>
      </div>
      <ul className="mt-5 grid gap-px overflow-hidden rounded-lg border border-white/10 bg-white/10 sm:grid-cols-2">
        {ALBUM_FIELDS.map((field) => {
          const value = data?.[field];
          return (
            <li
              className="flex items-center justify-between gap-3 bg-zinc-950 px-4 py-3"
              key={field}
            >
              <div className="min-w-0">
                <p className="text-sm font-medium text-zinc-100">
                  {ALBUM_FIELD_LABELS[field]}
                </p>
                {NOTES[field] && (
                  <p className="truncate text-xs text-zinc-500">
                    {NOTES[field]}
                  </p>
                )}
              </div>
              <div
                aria-label={`${ALBUM_FIELD_LABELS[field]} default source`}
                className="inline-flex shrink-0 rounded-full border border-white/10 bg-black/40 p-0.5"
                role="radiogroup"
              >
                {(
                  [
                    ["tags", "Tags", Tag],
                    ["online", "Online", Globe2],
                  ] as const
                ).map(([source, label, Icon]) => (
                  <button
                    aria-checked={value === source}
                    className={`flex items-center gap-1 rounded-full px-2.5 py-1 text-[11px] font-semibold transition-colors disabled:opacity-50 ${
                      value === source
                        ? "bg-white text-black"
                        : "text-zinc-400 hover:text-white"
                    }`}
                    disabled={!data || update.isPending}
                    key={source}
                    onClick={() => update.mutate({ [field]: source })}
                    role="radio"
                    type="button"
                  >
                    <Icon aria-hidden className="h-3 w-3" />
                    {label}
                  </button>
                ))}
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
