"use client";

import LibraryStatus from "@/features/library/library-status";
import AlbumCard from "@/features/library/album-card";
import SongCard from "@/features/library/song-card";
import { useSession } from "@/features/account/session-provider";
import { Button } from "@/components/ui/button";
import { homeFeedShouldShowSkeleton } from "@/features/library/library-readiness-state";
import { presentRecentItems } from "@/features/library/home-recent-items";
import {
  trackArtistCredit,
  trackArtistCreditParts,
} from "@/features/library/track-artist-credit";
import {
  emptyLibraryFeed,
  useLibraryFeed,
} from "@/features/library/use-library-feed";
import { getLibraryUnavailable } from "@parson/music-sdk";
import type { LibrarySong, ResponseAlbum } from "@parson/music-sdk/types";
import { Library } from "lucide-react";
import Link from "next/link";
import { Fragment, useEffect, useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  getMyLiveStations,
  getRecentLiveStations,
  DEFAULT_APPEARANCE,
  greetingForHour,
  visibleHomeRows,
  type HomeEssentials,
  type HomeRowId,
} from "@parson/music-sdk";
import { useOptionalAppearance } from "@/features/appearance/theme-provider";
import { useProfiles } from "@/features/profiles/use-profiles";
import { homeStats } from "@/features/library/home/home-stats";
import {
  ArtistCircle,
  BestYearsBand,
  DailyPicksBand,
  GenreTile,
  HomeAlbumCard,
  ShelfRow,
  StationTile,
  coverUrl,
} from "@/features/library/home/home-sections";

export default function LibraryFeed() {
  const { librarySetupPending, session } = useSession();
  const feedQuery = useLibraryFeed(Boolean(session));
  const feed = feedQuery.data ?? emptyLibraryFeed;
  const libraryReadiness = getLibraryUnavailable(feedQuery.error);
  const homeRows = visibleHomeRows(
    useOptionalAppearance()?.appearance ?? DEFAULT_APPEARANCE,
  );

  const recentSongs = feed.continue_listening;
  const recommendedSongs =
    feed.recommended.length > 0 ? feed.recommended : feed.shuffle;
  const recommendedAlbums = feed.albums;

  if (
    homeFeedShouldShowSkeleton(
      librarySetupPending,
      feedQuery.isPending,
      libraryReadiness?.state,
    )
  ) {
    return <HomeLoading />;
  }

  if (libraryReadiness) {
    return (
      <LibraryStatus
        readiness={libraryReadiness}
        onRetry={() => void feedQuery.refetch()}
      />
    );
  }

  if (feedQuery.isError) {
    return (
      <HomeMessage
        title="Mujin Music could not load your home feed"
        body="Your library is safe. Check the server connection and try again."
        action="Try again"
        onAction={() => void feedQuery.refetch()}
      />
    );
  }

  const hasContent =
    recentSongs.length > 0 ||
    recommendedSongs.length > 0 ||
    recommendedAlbums.length > 0 ||
    (feed.recently_added?.length ?? 0) > 0;
  const dailyPicks = feed.daily_picks?.picks ?? [];

  // Rows in the profile's order, hidden ones left out (Appearance → Home).
  const rows: Partial<Record<HomeRowId, () => ReactNode>> = {
    "daily-picks": () =>
      dailyPicks.length > 0 ? (
        <DailyPicksBand date={feed.daily_picks!.date} picks={dailyPicks} />
      ) : null,
    "recently-added": () =>
      (feed.recently_added?.length ?? 0) > 0 ? (
        <ShelfRow
          action={{ href: "/library", label: "All albums" }}
          eyebrow="Your library"
          title="Recently added"
        >
          {feed.recently_added!.map((album) => (
            <HomeAlbumCard album={album} key={album.id} />
          ))}
        </ShelfRow>
      ) : null,
    "recently-played": () =>
      recentSongs.length > 0 ? (
        <ShelfRow
          action={{ href: "/history", label: "History" }}
          eyebrow="Recent activity"
          title="Recently played"
        >
          <RecentRow songs={recentSongs} />
        </ShelfRow>
      ) : null,
    "new-for-you": () =>
      (feed.new_for_you?.length ?? 0) > 0 ? (
        <ShelfRow eyebrow="Newest releases you own" title="New for you">
          {feed.new_for_you!.map((album) => (
            <HomeAlbumCard album={album} key={album.id} />
          ))}
        </ShelfRow>
      ) : null,
    "similar-artists": () =>
      (feed.similar_artists?.length ?? 0) > 0 ? (
        <ShelfRow
          action={{ href: "/library?view=artists", label: "All artists" }}
          eyebrow={
            feed.similar_artists!.find((artist) => artist.reason)?.reason ??
            "From your library"
          }
          size="sm"
          title="Similar artists"
        >
          {feed.similar_artists!.map((artist) => (
            <ArtistCircle artist={artist} key={artist.id} />
          ))}
        </ShelfRow>
      ) : null,
    "best-years": () =>
      feed.best_years && feed.best_years.albums.length > 0 ? (
        <BestYearsBand prime={feed.best_years} />
      ) : null,
    "recommended-songs": () =>
      recommendedSongs.length > 0 ? (
        <ShelfRow eyebrow="Made for you" title="Recommended songs">
          <SongRow songs={recommendedSongs} />
        </ShelfRow>
      ) : null,
    genres: () =>
      (feed.genres?.length ?? 0) > 0 ? (
        <ShelfRow
          action={{ href: "/genre", label: "All genres" }}
          eyebrow="Browse"
          title="Genres"
        >
          {feed.genres!.map((genre) => (
            <GenreTile genre={genre} key={genre.slug} />
          ))}
        </ShelfRow>
      ) : null,
    "live-radio": () => <LiveRadioShelf />,
    "albums-you-might-like": () =>
      recommendedAlbums.length > 0 ? (
        <ShelfRow eyebrow="From your listening" title="Albums you might like">
          <AlbumRow albums={recommendedAlbums.slice(0, 12)} />
        </ShelfRow>
      ) : null,
  };

  return (
    <div className="relative min-h-full pb-36 md:pb-16">
      <HomeHero feed={feed} />
      <div className="mx-auto w-full max-w-[1480px] px-5 sm:px-8 lg:px-12">
        {!hasContent && (
          <HomeMessage
            title="Your music is ready"
            body="Browse your collection or add more music from Settings."
            action="Open library"
            href="/library"
          />
        )}
        <div className="space-y-14">
          {homeRows.map((row) => (
            <Fragment key={row}>{rows[row]?.()}</Fragment>
          ))}
        </div>
      </div>
    </div>
  );
}

/** Saved stations first, then recent ones; hidden when there are none. */
function LiveRadioShelf() {
  const { session } = useSession();
  const stations = useQuery({
    queryKey: ["home", "live-stations", session?.sub],
    queryFn: async () => {
      const [mine, recent] = await Promise.all([
        getMyLiveStations().catch(() => []),
        getRecentLiveStations().catch(() => []),
      ]);
      const seen = new Set<string>();
      return [...mine, ...recent].filter((station) => {
        if (seen.has(station.id)) return false;
        seen.add(station.id);
        return true;
      });
    },
    enabled: Boolean(session),
    staleTime: 60_000,
  });
  if (!stations.data?.length) return null;
  return (
    <ShelfRow
      action={{ href: "/radio", label: "Explore radio" }}
      eyebrow="On air"
      title="Live radio"
    >
      {stations.data.slice(0, 18).map((station) => (
        <StationTile key={station.id} station={station} />
      ))}
    </ShelfRow>
  );
}

/** Greeting by name, with what the library is made of. */
function HomeHero({ feed }: { feed: HomeEssentials }) {
  const { session } = useSession();
  const profiles = useProfiles();
  const current = profiles.data?.profiles.find(
    (profile) => profile.id === profiles.data?.currentId,
  );
  const name = firstName(current?.name || session?.username || "");
  const [greeting, setGreeting] = useState("Welcome back");
  useEffect(() => {
    setGreeting(greetingForHour(new Date().getHours()));
  }, []);
  const backdrop =
    feed.recently_added?.[0] ?? feed.daily_picks?.picks[0]?.album;
  const cover = backdrop
    ? coverUrl(backdrop.cover_url, backdrop.name, backdrop.artist.name)
    : null;
  const stats = homeStats(feed);
  return (
    <header className="relative isolate mb-12 overflow-hidden">
      {cover && (
        <div
          aria-hidden="true"
          className="mj-art-backdrop -z-20 opacity-35!"
          style={{ backgroundImage: `url("${cover}")` }}
        />
      )}
      <div aria-hidden="true" className="mj-hero-fade absolute inset-0 -z-10" />
      <div className="mx-auto w-full max-w-[1480px] px-5 pb-2 pt-10 sm:px-8 md:pt-14 lg:px-12">
        <p className="mj-eyebrow">{greeting}</p>
        <h1 className="mj-display mt-3 text-[2.75rem] text-mujin-text sm:text-[3.5rem] lg:text-[4rem]">
          {name ? `Hi, ${name}` : "Your library"}
        </h1>
        {stats.length > 0 && (
          <dl className="mt-9 grid grid-cols-2 gap-px overflow-hidden rounded-xl border border-white/[0.07] bg-white/[0.06] sm:grid-cols-4 xl:grid-cols-7 [&>*:last-child:nth-child(odd)]:col-span-2 sm:[&>*:last-child:nth-child(odd)]:col-span-1">
            {stats.map((stat) => (
              <div
                className="bg-mujin-base/80 px-5 py-4 backdrop-blur-md"
                key={stat.label}
              >
                <dt className="mj-eyebrow text-[0.625rem]!">
                  {/* "Lossless · 4 hi-res" breaks between its parts only,
                      never inside "hi-res". */}
                  {stat.label.split(" · ").map((part, index) => (
                    <Fragment key={part}>
                      {index > 0 ? " · " : ""}
                      <span className="whitespace-nowrap">{part}</span>
                    </Fragment>
                  ))}
                </dt>
                <dd className="mj-tnum mt-1.5 flex items-baseline gap-1.5">
                  <span className="mj-display text-[2.125rem] text-mujin-text">
                    {stat.value}
                  </span>
                  {stat.unit && (
                    <span className="text-[0.75rem] text-mujin-text-tertiary">
                      {stat.unit}
                    </span>
                  )}
                  {stat.accent && (
                    <span
                      aria-hidden="true"
                      className="mj-signal-dot ml-1 self-center"
                    />
                  )}
                </dd>
              </div>
            ))}
          </dl>
        )}
      </div>
    </header>
  );
}

function firstName(name: string) {
  const trimmed = name.trim();
  if (!trimmed) return "";
  const first = trimmed.split(/\s+/)[0]!;
  return first.charAt(0).toLocaleUpperCase() + first.slice(1);
}

function HomeLoading() {
  return (
    <div className="relative min-h-full py-7 pb-36">
      <div className="mx-auto w-full max-w-266 px-5 sm:px-7">
        <div className="mb-10 h-9 w-24 animate-pulse rounded-md bg-white/5" />
        <div className="space-y-11">
          {Array.from({ length: 3 }).map((_, row) => (
            <section className="mx-auto w-full max-w-250" key={row}>
              <div className="mb-4 h-5 w-44 animate-pulse rounded bg-white/5" />
              <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 sm:gap-4 lg:grid-cols-4 lg:gap-5">
                {Array.from({ length: 4 }).map((_, card) => (
                  <div className="min-w-0 space-y-3" key={card}>
                    <div className="aspect-square animate-pulse rounded-lg bg-white/4.5" />
                    <div className="h-3 w-3/4 animate-pulse rounded bg-white/5" />
                    <div className="h-3 w-1/2 animate-pulse rounded bg-white/[0.035]" />
                  </div>
                ))}
              </div>
            </section>
          ))}
        </div>
      </div>
    </div>
  );
}

function HomeMessage({
  action,
  body,
  href,
  onAction,
  title,
}: {
  action: string;
  body: string;
  href?: string;
  onAction?: () => void;
  title: string;
}) {
  return (
    <div className="grid min-h-64 place-items-center rounded-xl border border-dashed border-white/10 px-6 text-center">
      <div className="max-w-sm">
        <h2 className="text-lg font-semibold text-white">{title}</h2>
        <p className="mt-2 text-sm leading-6 text-zinc-500">{body}</p>
        <div className="mt-5 flex justify-center">
          {href ? (
            <Button asChild variant="outline">
              <Link href={href}>
                <Library /> {action}
              </Link>
            </Button>
          ) : (
            <Button onClick={onAction} variant="outline">
              {action}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}

function SongRow({ songs }: { songs: LibrarySong[] }) {
  return (
    <>
      {songs.map((song) => (
        <div key={song.id} className="min-w-0 snap-start">
          <SongCard
            song_name={song.name}
            song_id={song.id}
            path={song.path}
            artist_id={song.artist_object?.id}
            artist_name={songCardArtist(song)}
            artistCredits={songCardCredits(song)}
            album_id={song.album_object?.id}
            album_name={song.album_object?.name}
            album_cover={song.album_object?.cover_url}
          />
        </div>
      ))}
    </>
  );
}

function RecentRow({ songs }: { songs: LibrarySong[] }) {
  const items = presentRecentItems(songs);

  return (
    <>
      {items.map(({ historyIndex, kind, source: song }) => {
        const album = song.album_object;
        if (kind === "album") {
          return (
            <div
              key={`album-${song.id}-${historyIndex}`}
              className="min-w-0 snap-start"
            >
              <AlbumCard
                album_cover={album.cover_url}
                album_id={album.id}
                album_name={album.name}
                artist_id={song.artist_object.id}
                artist_name={song.artist_object.name}
                first_release_date={album.first_release_date}
                typeLabel="Album"
              />
            </div>
          );
        }
        return (
          <div
            key={`${song.id}-${historyIndex}`}
            className="min-w-0 snap-start"
          >
            <SongCard
              album_cover={song.album_object?.cover_url}
              album_id={song.album_object?.id}
              album_name={song.album_object?.name}
              artist_id={song.artist_object?.id}
              artist_name={songCardArtist(song)}
              artistCredits={songCardCredits(song)}
              path={song.path}
              song_id={song.id}
              song_name={song.name}
              typeLabel="Song"
            />
          </div>
        );
      })}
    </>
  );
}

function songCardCredits(song: LibrarySong) {
  return (
    trackArtistCreditParts({
      albumArtist: song.artist_object?.name ?? "",
      albumArtistId: song.artist_object?.id,
      albumType: song.album_object?.primary_type ?? "",
      contributingArtists: song.contributing_artists,
      contributingArtistIds: song.contributing_artist_ids,
      trackArtist: song.artist,
    }) ?? [
      {
        id: song.artist_object?.id,
        name: song.artist_object?.name ?? song.artist,
      },
    ]
  );
}

function songCardArtist(song: LibrarySong) {
  return (
    trackArtistCredit({
      albumArtist: song.artist_object?.name ?? "",
      albumType: song.album_object?.primary_type ?? "",
      contributingArtists: song.contributing_artists,
      trackArtist: song.artist,
    }) ??
    song.artist_object?.name ??
    song.artist
  );
}

function AlbumRow({ albums }: { albums: ResponseAlbum[] }) {
  return (
    <>
      {albums.map((album) => (
        <div key={album.id} className="min-w-0 snap-start">
          <AlbumCard
            artist_id={album.artist_object.id}
            artist_name={album.artist_object.name}
            album_id={album.id}
            album_name={album.name}
            album_cover={album.cover_url}
            first_release_date={album.first_release_date}
          />
        </div>
      ))}
    </>
  );
}
