"use client";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  editorialSourceName,
  editorialSourceNativeName,
  enrichmentStageLine,
  getEnrichment,
  moveEditorialSource,
  toggleEditorialSource,
  updateEnrichment,
  type EditorialSource,
  type EnrichmentSettings as Settings,
  type GenreSource,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronDown, ChevronUp, Globe2, Loader2 } from "lucide-react";
import { toast } from "sonner";

const LANGUAGES: [code: string, name: string][] = [
  ["en", "English"],
  ["de", "Deutsch"],
  ["es", "Español"],
  ["fr", "Français"],
  ["it", "Italiano"],
  ["ja", "日本語"],
  ["nl", "Nederlands"],
  ["pl", "Polski"],
  ["pt", "Português"],
  ["sv", "Svenska"],
];

/**
 * Roon's "Editorial sources": each Wikipedia language on or off, in the
 * order pages use them. The first one on is the main source; the others
 * are fetched too, so album and artist pages can switch text source.
 */
function EditorialSources({
  sources,
  disabled,
  onChange,
}: {
  sources: EditorialSource[];
  disabled: boolean;
  onChange: (sources: EditorialSource[]) => void;
}) {
  const main = sources.find((source) => source.enabled)?.id;
  return (
    <div className="space-y-2">
      <div>
        <p className="font-medium text-zinc-200">Editorial sources</p>
        <p className="mt-0.5 text-xs leading-5 text-zinc-500">
          Biographies, album background and reviews come from the first source
          turned on. The others are fetched too, so album and artist pages can
          switch to them. Use the arrows to set the order.
        </p>
      </div>
      <ol
        aria-label="Editorial sources in priority order"
        className="divide-y divide-white/6 overflow-hidden rounded-md border border-white/8"
      >
        {sources.map((source, index) => {
          const native = editorialSourceNativeName(source.id);
          const name = editorialSourceName(source.id);
          return (
            <li
              className={`flex items-center gap-3 px-3 py-2 ${
                source.enabled ? "bg-white/[0.03]" : ""
              }`}
              key={source.id}
            >
              <span className="w-5 text-right text-xs tabular-nums text-zinc-600">
                {index + 1}
              </span>
              <Checkbox
                aria-label={`Use ${name}`}
                checked={source.enabled}
                disabled={
                  disabled ||
                  (source.enabled &&
                    source.id === main &&
                    sources.filter((item) => item.enabled).length === 1)
                }
                onCheckedChange={() =>
                  onChange(toggleEditorialSource(sources, source.id))
                }
              />
              <span
                className={`min-w-0 flex-1 truncate ${
                  source.enabled ? "text-zinc-100" : "text-zinc-500"
                }`}
              >
                {name}
                {native && (
                  <span className="ml-1.5 text-xs text-zinc-500">{native}</span>
                )}
              </span>
              {source.id === main && (
                <span className="rounded-full bg-white/10 px-2 py-0.5 text-[0.625rem] font-semibold uppercase tracking-[0.1em] text-zinc-300">
                  Main
                </span>
              )}
              <span className="flex shrink-0">
                <button
                  aria-label={`Move ${name} up`}
                  className="rounded p-1 text-zinc-500 transition-colors hover:bg-white/8 hover:text-white disabled:opacity-30 disabled:hover:bg-transparent"
                  disabled={disabled || index === 0}
                  onClick={() =>
                    onChange(moveEditorialSource(sources, source.id, -1))
                  }
                  type="button"
                >
                  <ChevronUp className="h-4 w-4" />
                </button>
                <button
                  aria-label={`Move ${name} down`}
                  className="rounded p-1 text-zinc-500 transition-colors hover:bg-white/8 hover:text-white disabled:opacity-30 disabled:hover:bg-transparent"
                  disabled={disabled || index === sources.length - 1}
                  onClick={() =>
                    onChange(moveEditorialSource(sources, source.id, 1))
                  }
                  type="button"
                >
                  <ChevronDown className="h-4 w-4" />
                </button>
              </span>
            </li>
          );
        })}
      </ol>
    </div>
  );
}

/**
 * Online metadata: Mujin Music contacts MusicBrainz and Wikimedia only while
 * it is on. First-run setup turns it on unless the person unticks it there.
 */
export default function EnrichmentSettings() {
  const queryClient = useQueryClient();
  const overview = useQuery({
    queryKey: ["metadata-enrichment"],
    queryFn: getEnrichment,
    refetchInterval: (query) =>
      query.state.data?.enabled &&
      (query.state.data.counts.pending > 0 ||
        query.state.data.stages?.some((stage) => stage.counts.pending > 0))
        ? 10_000
        : false,
  });
  const update = useMutation({
    mutationFn: (patch: Partial<Settings>) => updateEnrichment(patch),
    onSuccess: (data) => {
      queryClient.setQueryData(["metadata-enrichment"], data);
      void queryClient.invalidateQueries({ queryKey: ["album-about"] });
      void queryClient.invalidateQueries({ queryKey: ["artist-about"] });
      void queryClient.invalidateQueries({ queryKey: ["genre-tree"] });
      void queryClient.invalidateQueries({ queryKey: ["genre-page"] });
    },
    onError: () => toast("Could not change online metadata settings."),
  });
  const data = overview.data;
  const total = data
    ? data.counts.pending +
      data.counts.done +
      data.counts.skipped +
      data.counts.failed
    : 0;
  const finished = data ? total - data.counts.pending : 0;

  return (
    <section className="border-t border-white/[0.08] pt-7">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 className="flex items-center gap-2 text-base font-semibold text-white">
            <Globe2 aria-hidden className="h-4 w-4 text-zinc-400" />
            Online metadata
          </h2>
          <p className="mt-1 max-w-xl text-sm leading-6 text-zinc-500">
            Adds credits and composers&apos; full catalogues from MusicBrainz;
            biographies, album background, critical reception and review scores
            from Wikipedia, in every language you turn on below; notable works,
            influences and followers from Wikidata; front covers from the Cover
            Art Archive for albums without artwork, with their sources and
            licenses shown; and lyrics from LRCLIB for tracks whose files carry
            none. Lyrics in your files (embedded LRC, SYLT/USLT, or an .lrc
            beside the track) always show. First-run setup turns it on unless
            you untick it there; only artist and album names leave this server.
          </p>
        </div>
        <Button
          aria-pressed={Boolean(data?.enabled)}
          disabled={!data || update.isPending}
          onClick={() => update.mutate({ enabled: !data?.enabled })}
          variant={data?.enabled ? "outline" : "default"}
        >
          {update.isPending && <Loader2 className="h-4 w-4 animate-spin" />}
          {data?.enabled ? "Turn off" : "Turn on"}
        </Button>
      </div>

      <div className="mt-5 space-y-4 rounded-lg border border-white/10 p-4 text-sm">
        <p className="leading-6 text-zinc-400">
          When on, the server sends album titles, artist names and MusicBrainz
          ids (and, for lyrics, the open track&apos;s title, artist, album and
          length) to{" "}
          {(
            data?.sources ?? ["musicbrainz.org", "wikipedia.org", "lrclib.net"]
          ).join(", ")}
          . MusicBrainz is asked at most once a second, and each service has its
          own queue, so a busy one never holds up the others. Nothing about your
          listening is sent.
        </p>
        {data?.enabled && (
          <p className="text-zinc-300" role="status">
            {data.counts.pending > 0
              ? `Looked up ${finished.toLocaleString()} of ${total.toLocaleString()} albums and artists…`
              : `${data.counts.done.toLocaleString()} albums and artists matched${
                  data.counts.skipped
                    ? `, ${data.counts.skipped.toLocaleString()} without a confident match`
                    : ""
                }${data.counts.failed ? `, ${data.counts.failed.toLocaleString()} failed` : ""}.`}
          </p>
        )}
        {data?.enabled && data.stages && data.stages.length > 0 && (
          <ul
            aria-label="Lookup queues"
            className="grid gap-1.5 text-xs text-zinc-500"
          >
            {data.stages.map((stage) => {
              const line = enrichmentStageLine(stage);
              return (
                <li className="flex flex-wrap gap-x-2" key={stage.stage}>
                  <span className="font-medium text-zinc-300">
                    {line.label}
                  </span>
                  <span className={line.busy ? "text-amber-300/90" : ""}>
                    {line.detail}
                  </span>
                </li>
              );
            })}
          </ul>
        )}
        {data?.editorial_sources?.length ? (
          <EditorialSources
            disabled={update.isPending}
            onChange={(editorial_sources) =>
              update.mutate({ editorial_sources })
            }
            sources={data.editorial_sources}
          />
        ) : null}
        <div className="flex flex-wrap items-center gap-x-8 gap-y-3">
          <label
            className={`flex items-center gap-2 text-zinc-300 ${
              data?.editorial_sources?.length ? "hidden" : ""
            }`}
          >
            <span>Wikipedia language</span>
            <select
              className="h-9 rounded-md border border-zinc-700 bg-zinc-900 px-2 text-zinc-100"
              disabled={!data || update.isPending}
              onChange={(event) =>
                update.mutate({ language: event.target.value })
              }
              value={data?.language ?? "en"}
            >
              {LANGUAGES.map(([code, name]) => (
                <option key={code} value={code}>
                  {name}
                </option>
              ))}
            </select>
          </label>
          <label className="flex items-center gap-2 text-zinc-300">
            <Checkbox
              checked={data?.artist_images ?? true}
              disabled={!data || update.isPending}
              onCheckedChange={(checked) =>
                update.mutate({ artist_images: checked === true })
              }
            />
            Freely licensed artist photos from Wikimedia Commons
          </label>
          <label className="flex items-center gap-2 text-zinc-300">
            <span>Genres from</span>
            <select
              aria-describedby="genre-source-note"
              className="h-9 rounded-md border border-zinc-700 bg-zinc-900 px-2 text-zinc-100"
              disabled={!data || update.isPending}
              onChange={(event) =>
                update.mutate({
                  genre_source: event.target.value as GenreSource,
                })
              }
              value={data?.genre_source ?? "online"}
            >
              <option value="online">Online (MusicBrainz)</option>
              <option value="tags">File tags</option>
            </select>
          </label>
        </div>
        <p className="text-xs leading-5 text-zinc-500" id="genre-source-note">
          Each falls back to the other when an album has none; genres you edit
          on an album page always win.
          {!data?.enabled && " Online genres need online metadata turned on."}
        </p>
      </div>
    </section>
  );
}
