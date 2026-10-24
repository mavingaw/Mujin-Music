"use client";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  getEnrichment,
  updateEnrichment,
  type EnrichmentSettings as Settings,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Globe2, Loader2 } from "lucide-react";
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
 * Online metadata is off by default: Parson only contacts MusicBrainz and
 * Wikimedia after an administrator turns it on here.
 */
export default function EnrichmentSettings() {
  const queryClient = useQueryClient();
  const overview = useQuery({
    queryKey: ["metadata-enrichment"],
    queryFn: getEnrichment,
    refetchInterval: (query) =>
      query.state.data?.enabled && query.state.data.counts.pending > 0
        ? 10_000
        : false,
  });
  const update = useMutation({
    mutationFn: (patch: Partial<Settings>) => updateEnrichment(patch),
    onSuccess: (data) => {
      queryClient.setQueryData(["metadata-enrichment"], data);
      void queryClient.invalidateQueries({ queryKey: ["album-about"] });
      void queryClient.invalidateQueries({ queryKey: ["artist-about"] });
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
            Adds credits from MusicBrainz, and biographies, album background and
            critical reception from Wikipedia, with their sources and licenses
            shown. Off by default.
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
          ids to{" "}
          {(data?.sources ?? ["musicbrainz.org", "wikipedia.org"]).join(", ")}.
          MusicBrainz is asked at most once a second. Nothing about your
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
        <div className="flex flex-wrap items-center gap-x-8 gap-y-3">
          <label className="flex items-center gap-2 text-zinc-300">
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
        </div>
      </div>
    </section>
  );
}
