"use client";

import {
  SOURCE_PREFERENCES,
  SOURCE_PREFERENCE_LABELS,
  STREAMING_PROVIDER_NAMES,
  albumPlayChoice,
  findAlbumVersions,
  getSourcePreference,
  rankAlbumVersions,
  setSourcePreference,
  type AlbumVersion,
  type AlbumVersionSubject,
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

/** A streaming album page's album, as the subject of a versions search. */
export function streamingAlbumSubject(
  album: StreamingAlbum,
): AlbumVersionSubject {
  return {
    id: album.id,
    title: album.title,
    artistName: album.artist.name,
    source: album.provider,
    format: album.format ?? null,
  };
}

export function albumVersionsKey(subjectId: string, connected: string[]) {
  return ["streaming", "versions", subjectId, connected.join(",")] as const;
}

/**
 * Every place this album lives (local files, TIDAL, Qobuz), ordered by the
 * listener's source preference. Nothing is searched until a service is
 * connected: with none, this page's copy is the only one.
 */
export function useAlbumVersions(subject: AlbumVersionSubject | null) {
  const { connected } = useStreamingProviders();
  const preference = useSourcePreference();
  const providers = connected.map((provider) => provider.id);
  const versions = useQuery({
    queryKey: albumVersionsKey(subject?.id ?? "", providers),
    queryFn: (): Promise<AlbumVersion[]> =>
      findAlbumVersions(subject!, providers),
    enabled: Boolean(subject) && providers.length > 0,
    staleTime: 5 * 60_000,
  });
  const current = preference.data ?? "highest_quality";
  const ranked = versions.data ? rankAlbumVersions(versions.data, current) : [];
  return {
    ranked,
    preference: current,
    /** True while the copies are still being looked up. */
    pending: Boolean(subject) && providers.length > 0 && versions.isPending,
    /** What album-level Play starts; null when this is the only copy. */
    choice: subject ? albumPlayChoice(subject.id, ranked) : null,
    /** Resolves the ranking for a Play pressed before it was ready. */
    async resolve(): Promise<RankedAlbumVersion[]> {
      if (!subject || !providers.length) return [];
      if (versions.data) return ranked;
      const settled = await Promise.race([
        versions.refetch().then((result) => result.data ?? null),
        new Promise<null>((resolve) => setTimeout(() => resolve(null), 4_000)),
      ]);
      return settled ? rankAlbumVersions(settled, current) : [];
    },
  };
}

function sourceName(version: RankedAlbumVersion) {
  return version.source === "local"
    ? "your library"
    : STREAMING_PROVIDER_NAMES[version.source];
}

/** Points at the copy the preference would play when it is not this one. */
export function BetterVersionNotice({
  subject,
}: {
  subject: AlbumVersionSubject;
}) {
  const { ranked } = useAlbumVersions(subject);
  const best = ranked[0];
  if (!best || ranked.length < 2 || best.id === subject.id) return null;
  return (
    <Link
      className="mb-6 flex items-center gap-3 rounded-xl border border-white/10 bg-white/4 px-4 py-3 text-sm text-zinc-300 transition-colors hover:bg-white/7"
      href={`/album?id=${encodeURIComponent(best.id)}`}
    >
      <span className="min-w-0 flex-1">
        Play starts your preferred copy, in {sourceName(best)}
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

export function AlbumVersions({ subject }: { subject: AlbumVersionSubject }) {
  const album = subject;
  const { ranked, preference } = useAlbumVersions(subject);
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
        Mujin Music marks one copy to play when the same album is in your files,
        on TIDAL or on Qobuz. Highest quality compares bit depth and sample
        rate; ties go to your own files.
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
