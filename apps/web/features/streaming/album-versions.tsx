"use client";

import {
  SOURCE_PREFERENCES,
  SOURCE_PREFERENCE_LABELS,
  STREAMING_PROVIDER_NAMES,
  getAlbumInfo,
  getSourcePreference,
  localAlbumFormat,
  rankAlbumVersions,
  searchLibrary,
  searchStreaming,
  setSourcePreference,
  type AlbumVersion,
  type RankedAlbumVersion,
  type StreamingAlbum,
  type StreamingSourcePreference,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowRight } from "lucide-react";
import Link from "next/link";
import { FormatBadge, LocalSourceBadge, SourceBadge } from "./source-badge";
import { formatTone } from "./streaming-model";
import { useStreamingProviders } from "./use-streaming";

export const sourcePreferenceKey = ["streaming", "source-preference"] as const;

export function useSourcePreference() {
  return useQuery({
    queryKey: sourcePreferenceKey,
    queryFn: getSourcePreference,
    staleTime: 5 * 60_000,
  });
}

const comparable = (value: string) =>
  value
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/\(.*?\)|\[.*?\]/g, "")
    .replace(/[^a-z0-9]+/g, " ")
    .trim();

/**
 * Every place this album lives (this service, the other one, local files),
 * ordered by the listener's source preference.
 */
export function useAlbumVersions(album: StreamingAlbum) {
  const { connected } = useStreamingProviders();
  const preference = useSourcePreference();
  const others = connected
    .map((provider) => provider.id)
    .filter((provider) => provider !== album.provider);
  const query = `${album.artist.name} ${album.title}`;
  const versions = useQuery({
    queryKey: ["streaming", "versions", album.id, others.join(",")],
    queryFn: async (): Promise<AlbumVersion[]> => {
      const [remote, local] = await Promise.all([
        others.length
          ? searchStreaming(query, { providers: others, limit: 10 })
          : Promise.resolve({ query, services: [] }),
        searchLibrary(album.title).catch(() => []),
      ]);
      const title = comparable(album.title);
      const artist = comparable(album.artist.name);
      const streaming = remote.services.flatMap((service) =>
        (service.results?.albums ?? []).filter(
          (candidate) =>
            comparable(candidate.title) === title &&
            comparable(candidate.artist.name) === artist,
        ),
      );
      const localAlbums = local.filter(
        (item) =>
          item.item_type === "album" &&
          comparable(item.name) === title &&
          comparable(item.artist_object?.name ?? "") === artist,
      );
      // The local files' format comes from the indexer's technical read.
      const localVersions = await Promise.all(
        localAlbums.map(async (item): Promise<AlbumVersion> => {
          const info = await getAlbumInfo(item.id, false).catch(() => null);
          const songs =
            info && "songs" in info && Array.isArray(info.songs)
              ? info.songs
              : [];
          return {
            id: item.id,
            title: item.name,
            source: "local",
            format: localAlbumFormat(songs),
          };
        }),
      );
      return [
        {
          id: album.id,
          title: album.title,
          source: album.provider,
          format: album.format ?? null,
        },
        ...streaming.map((version): AlbumVersion => ({
          id: version.id,
          title: version.title,
          source: version.provider,
          format: version.format ?? null,
        })),
        ...localVersions,
      ];
    },
    staleTime: 5 * 60_000,
  });
  const ranked = versions.data
    ? rankAlbumVersions(versions.data, preference.data ?? "highest_quality")
    : [];
  return { ranked, preference: preference.data ?? "highest_quality" };
}

function sourceName(version: RankedAlbumVersion) {
  return version.source === "local"
    ? "your library"
    : STREAMING_PROVIDER_NAMES[version.source];
}

/** Points at the copy the preference would play when it is not this one. */
export function BetterVersionNotice({ album }: { album: StreamingAlbum }) {
  const { ranked } = useAlbumVersions(album);
  const best = ranked[0];
  if (!best || ranked.length < 2 || best.id === album.id) return null;
  return (
    <Link
      className="mb-6 flex items-center gap-3 rounded-xl border border-white/10 bg-white/4 px-4 py-3 text-sm text-zinc-300 transition-colors hover:bg-white/7"
      href={`/album?id=${encodeURIComponent(best.id)}`}
    >
      <span className="min-w-0 flex-1">
        Your preferred copy is in {sourceName(best)}
        {best.format?.label ? ` (${best.format.label})` : ""}.{" "}
        <span className="text-zinc-500">{best.reason}</span>
      </span>
      <span className="flex shrink-0 items-center gap-1 font-medium text-white">
        Open
        <ArrowRight className="h-4 w-4" />
      </span>
    </Link>
  );
}

export function AlbumVersions({ album }: { album: StreamingAlbum }) {
  const { ranked, preference } = useAlbumVersions(album);
  if (ranked.length < 2) return null;
  return (
    <section className="mt-10">
      <div className="mb-3 flex flex-wrap items-baseline justify-between gap-2">
        <h2 className="text-lg font-semibold text-white">Versions</h2>
        <Link
          className="text-xs text-zinc-500 hover:text-zinc-300"
          href="/settings?tab=services"
        >
          Plays: {SOURCE_PREFERENCE_LABELS[preference]}
        </Link>
      </div>
      <div className="divide-y divide-white/6 rounded-xl border border-white/8">
        {ranked.map((version) => (
          <Link
            aria-current={version.id === album.id ? "page" : undefined}
            className="flex flex-wrap items-center gap-x-4 gap-y-1 px-4 py-3 text-sm transition-colors hover:bg-white/3"
            href={`/album?id=${encodeURIComponent(version.id)}`}
            key={version.id}
          >
            <span className="w-28 shrink-0">
              {version.source === "local" ? (
                <LocalSourceBadge />
              ) : (
                <SourceBadge provider={version.source} />
              )}
            </span>
            <span className="min-w-0 flex-1 truncate text-zinc-200">
              {version.title}
            </span>
            {version.format?.label && (
              <FormatBadge
                label={version.format.label}
                tone={formatTone(version.format)}
              />
            )}
            {version.preferred && (
              <span
                className="rounded-full bg-emerald-400/15 px-2 py-0.5 text-xs font-medium text-emerald-300"
                title={version.reason ?? undefined}
              >
                Plays
              </span>
            )}
            {version.id === album.id && (
              <span className="text-xs text-zinc-500">Viewing</span>
            )}
          </Link>
        ))}
      </div>
      {ranked[0]?.reason && (
        <p className="mt-2 text-xs text-zinc-500">{ranked[0].reason}</p>
      )}
    </section>
  );
}

/** Settings control for which copy plays. */
export function SourcePreferenceSetting() {
  const client = useQueryClient();
  const preference = useSourcePreference();
  const save = useMutation({
    mutationFn: (source: StreamingSourcePreference) =>
      setSourcePreference(source),
    onSuccess: (source) => client.setQueryData(sourcePreferenceKey, source),
  });
  const current = preference.data ?? "highest_quality";
  return (
    <section className="rounded-xl border border-white/8 p-4">
      <h3 className="text-sm font-semibold text-white">
        When an album is in more than one place
      </h3>
      <p className="mt-1 text-xs text-zinc-400">
        Parson marks one copy to play when the same album is in your files, on
        TIDAL or on Qobuz. Highest quality compares bit depth and sample rate;
        ties go to your own files.
      </p>
      <div
        aria-label="Preferred source"
        className="mt-3 flex flex-wrap gap-2"
        role="radiogroup"
      >
        {SOURCE_PREFERENCES.map((source) => (
          <button
            aria-checked={current === source}
            className={`rounded-full border px-3 py-1.5 text-xs font-medium transition-colors ${
              current === source
                ? "border-white bg-white text-black"
                : "border-white/15 text-zinc-300 hover:border-white/40"
            }`}
            disabled={save.isPending || preference.isPending}
            key={source}
            onClick={() => save.mutate(source)}
            role="radio"
            type="button"
          >
            {SOURCE_PREFERENCE_LABELS[source]}
          </button>
        ))}
      </div>
      {save.isError && (
        <p className="mt-2 text-xs text-red-400">Could not save. Try again.</p>
      )}
    </section>
  );
}
