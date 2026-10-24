"use client";

import {
  browseStreaming,
  type StreamingBrowseSection,
  type StreamingProviderId,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { useState } from "react";
import { usePageTitle } from "@/components/app/title-metadata";
import { cn } from "@/lib/cn";
import AlbumCard from "@/features/library/album-card";
import { ProviderMark } from "./source-badge";
import { SandboxNotice } from "./services-settings";
import { StreamingTrackList } from "./library-sources";
import { useStreamingProviders } from "./use-streaming";

const sections: { id: StreamingBrowseSection; label: string }[] = [
  { id: "new_releases", label: "New releases" },
  { id: "favorite_albums", label: "Favorite albums" },
  { id: "favorite_tracks", label: "Favorite tracks" },
];

function BrowseSectionView({
  provider,
  section,
}: {
  provider: StreamingProviderId;
  section: StreamingBrowseSection;
}) {
  const page = useQuery({
    queryKey: ["streaming", "browse", provider, section],
    queryFn: () => browseStreaming(provider, section, { limit: 50 }),
  });
  if (page.isPending) {
    return (
      <p className="flex items-center gap-2 text-sm text-zinc-500">
        <Loader2 className="h-4 w-4 animate-spin" /> Loading…
      </p>
    );
  }
  if (page.isError || !page.data) {
    return (
      <p className="text-sm text-zinc-400">
        This section could not be loaded.{" "}
        <button
          className="text-white underline-offset-4 hover:underline"
          onClick={() => void page.refetch()}
          type="button"
        >
          Try again
        </button>
      </p>
    );
  }
  if (section === "favorite_tracks") {
    return page.data.tracks.length ? (
      <StreamingTrackList tracks={page.data.tracks} />
    ) : (
      <p className="text-sm text-zinc-500">No favorite tracks yet.</p>
    );
  }
  if (!page.data.albums.length) {
    return <p className="text-sm text-zinc-500">Nothing here yet.</p>;
  }
  return (
    <div className="grid grid-cols-2 gap-x-4 gap-y-10 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
      {page.data.albums.map((album) => (
        <AlbumCard
          album_cover={album.cover_url ?? ""}
          album_id={album.id}
          album_name={album.title}
          artist_id={album.artist.id}
          artist_name={album.artist.name}
          first_release_date={album.release_date ?? ""}
          formatLabel={album.format?.label}
          key={album.id}
          source={album.provider}
        />
      ))}
    </div>
  );
}

/** Each connected service's own browser, as in Roon's navigation. */
export default function StreamingBrowse() {
  usePageTitle("Streaming");
  const requested = useSearchParams().get("provider");
  const { connected, sandbox, isPending } = useStreamingProviders();
  const [chosen, setChosen] = useState<StreamingProviderId | null>(
    requested === "tidal" || requested === "qobuz" ? requested : null,
  );
  const [section, setSection] =
    useState<StreamingBrowseSection>("new_releases");
  const active =
    connected.find((provider) => provider.id === chosen) ?? connected[0];

  return (
    <section className="mx-auto w-full max-w-266 space-y-7 px-5 py-8 pb-36 sm:px-7">
      <header className="flex flex-wrap items-end justify-between gap-4">
        <h1 className="text-3xl font-semibold text-white">Streaming</h1>
        {connected.length > 1 && (
          <div className="flex gap-2" role="tablist" aria-label="Services">
            {connected.map((provider) => (
              <button
                aria-selected={active?.id === provider.id}
                className={cn(
                  "inline-flex h-9 items-center gap-2 rounded-full border px-4 text-sm font-semibold",
                  active?.id === provider.id
                    ? "border-white bg-white text-black"
                    : "border-white/12 text-zinc-300 hover:text-white",
                )}
                key={provider.id}
                onClick={() => setChosen(provider.id)}
                role="tab"
                type="button"
              >
                <ProviderMark
                  onLight={active?.id === provider.id}
                  provider={provider.id}
                />
                {provider.name}
              </button>
            ))}
          </div>
        )}
      </header>
      {sandbox && <SandboxNotice compact />}
      {!isPending && !active && (
        <div className="rounded-xl border border-white/8 px-6 py-14 text-center">
          <p className="text-lg font-semibold text-white">
            Connect TIDAL or Qobuz
          </p>
          <p className="mx-auto mt-2 max-w-md text-sm leading-6 text-zinc-400">
            Sign in with your own account and credentials to browse new releases
            and favorites, and to add streaming albums to your library.
          </p>
          <Link
            className="mt-6 inline-flex h-10 items-center rounded-full bg-white px-6 text-sm font-semibold text-black"
            href="/settings?tab=services"
          >
            Open Services
          </Link>
        </div>
      )}
      {active && (
        <>
          <nav
            aria-label={`${active.name} sections`}
            className="flex gap-1 border-b border-white/8"
          >
            {sections.map((item) => (
              <button
                aria-current={section === item.id ? "page" : undefined}
                className={cn(
                  "relative h-11 px-4 text-sm font-medium",
                  section === item.id
                    ? "text-white"
                    : "text-zinc-500 hover:text-zinc-200",
                )}
                key={item.id}
                onClick={() => setSection(item.id)}
                type="button"
              >
                {item.label}
                {section === item.id && (
                  <span className="absolute inset-x-3 bottom-0 h-0.5 bg-white" />
                )}
              </button>
            ))}
          </nav>
          <BrowseSectionView
            key={`${active.id}-${section}`}
            provider={active.id}
            section={section}
          />
        </>
      )}
    </section>
  );
}
