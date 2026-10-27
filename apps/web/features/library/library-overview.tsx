"use client";

import { Button } from "@/components/ui/button";
import AlbumCard from "@/features/library/album-card";
import ArtistMenu from "@/features/library/artist-menu";
import { artistCatalogSummary } from "@/features/library/artist-catalog-summary";
import CreatePlaylistDialog from "@/features/library/create-playlist-dialog";
import PlaylistCover from "@/features/library/playlist-cover";
import PlaylistMenu from "@/features/library/playlist-menu";
import SongMenu from "@/features/library/song-menu";
import BannedView from "@/features/library/banned-view";
import { BannedBadge } from "@/features/library/radio-bans";
import { SongTitle, ExplicitBadge } from "./song-title";
import { isExplicitSongTitle } from "./song-presentation";
import { ArtistCreditLinks } from "./artist-credit-links";
import { catalogSongArtistCredits } from "./track-artist-credit";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { useZonePlayback } from "@/features/player/zones/use-zone-playback";
import getBaseURL from "@/lib/api/server-url";
import { defaultCover } from "@/lib/images/default-cover";
import {
  composerFacts,
  decodeFocus,
  emptyFocus,
  encodeFocus,
  getCatalogAlbums,
  getFocusPage,
  getLibraryCatalog,
  getLibraryCatalogArtists,
  getPlaylists,
  isFocused,
  withEntity,
  type CatalogFilter,
  type CatalogSort,
  type FocusComposer,
  type FocusEntity,
  type FocusQuery,
  type LibraryCatalogAlbum,
  type LibraryCatalogArtist,
  type LibraryCatalogSong,
} from "@parson/music-sdk";
import {
  keepPreviousData,
  useInfiniteQuery,
  useQuery,
} from "@tanstack/react-query";
import {
  Ban,
  Disc3,
  ChevronRight,
  Feather,
  ListMusic,
  Heart,
  Import,
  Music2,
  Pause,
  Play,
  Plus,
  ScrollText,
  UserRound,
  Wrench,
} from "lucide-react";
import { useSession } from "@/features/account/session-provider";
import Image from "next/image";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import FocusBar from "./focus/focus-bar";
import FocusPanel from "./focus/focus-panel";
import SmartPlaylistDialog from "./focus/smart-playlist-dialog";
import {
  audioBadge,
  focusCountLabel,
  focusQueryKey,
  qualityBadgeLabel,
  usesFocusEndpoint,
} from "./focus/focus-state";
import { usePlayFocus } from "./focus/use-play-focus";
import CompositionList from "./composition-list";
import { composerInitials, composerSummary } from "./composer-summary";
import { uniqueById } from "./library-items";
import {
  InfiniteLoad,
  LibraryLoading,
  LibraryMessage,
  formatDuration,
} from "./library-view-primitives";
import {
  SourceFilterChips,
  StreamingLibraryEmpty,
  StreamingTrackList,
  useLibrarySources,
} from "@/features/streaming/library-sources";
import { SavedPlaylistCopyCards } from "@/features/streaming/streaming-playlists";
import type { LibraryAlbumEntry } from "@/features/streaming/streaming-model";
import {
  activeFocusCount,
  catalogAlbumEntry,
  emptyFocus as emptyCatalogFocus,
  parseCatalogSort,
  parseLibraryFocus,
  toCatalogFilter,
  writeCatalogSort,
  writeLibraryFocus,
  type LibraryFocus as CatalogFocus,
} from "@/features/streaming/library-focus";
import { LibraryFocusBar as CatalogFocusBar } from "@/features/streaming/library-focus-bar";

const PAGE_SIZE = 100;
const views = [
  "albums",
  "songs",
  "artists",
  "composers",
  "compositions",
  "playlists",
  "banned",
] as const;
type LibraryView = (typeof views)[number];
type BrowseView = Exclude<LibraryView, "playlists" | "banned">;

function isBrowseView(view: LibraryView): view is BrowseView {
  return view !== "playlists" && view !== "banned";
}

type FocusedAlbum = LibraryCatalogAlbum & { quality?: string | null };

/** A local album the Focus matched, as a library grid entry. */
function focusedAlbumEntry(album: FocusedAlbum): LibraryAlbumEntry {
  return {
    key: `local:${album.id}`,
    id: album.id,
    name: album.name,
    artistId: album.artistId,
    artistName: album.artistName,
    cover: album.coverPath,
    year: album.releaseYear,
    source: null,
    formatLabel: album.quality === "hires" ? qualityBadgeLabel("hires") : null,
  };
}
type FocusedSong = LibraryCatalogSong & { badge?: string | null };

function entityForView(view: LibraryView): FocusEntity {
  return isBrowseView(view) ? view : "albums";
}

function libraryURL(view: LibraryView, focus: FocusQuery | null): string {
  const next = new URLSearchParams(window.location.search);
  if (view === "albums") next.delete("view");
  else next.set("view", view);
  if (focus && isBrowseView(view) && usesFocusEndpoint(focus)) {
    next.set("focus", encodeFocus(focus));
  } else {
    next.delete("focus");
  }
  return `/library${next.size ? `?${next}` : ""}`;
}

function writeURL(view: LibraryView, focus: FocusQuery | null) {
  window.history.replaceState(
    window.history.state,
    "",
    libraryURL(view, focus),
  );
}

export default function LibraryOverview() {
  const { session } = useSession();
  const searchParams = useSearchParams();
  const requestedView = searchParams.get("view");
  const [view, setView] = useState<LibraryView>(() =>
    views.includes(requestedView as LibraryView)
      ? (requestedView as LibraryView)
      : "albums",
  );
  const [focus, setFocusState] = useState<FocusQuery>(() =>
    decodeFocus(
      searchParams.get("focus"),
      entityForView(
        views.includes(requestedView as LibraryView)
          ? (requestedView as LibraryView)
          : "albums",
      ),
    ),
  );
  const [panelOpen, setPanelOpen] = useState(false);
  const [smartOpen, setSmartOpen] = useState(false);
  const playFocusQuery = usePlayFocus();
  const browsing = isBrowseView(view);
  const focusActive = browsing && usesFocusEndpoint(focus);
  const catalogEnabled = !focusActive && view === "songs";
  // The storage-location focus (local files, TIDAL or Qobuz library), with
  // genre, decade and order, over the albums catalog that also holds
  // streaming library albums. The full Focus above narrows the local
  // library; while it is active, it decides what is shown.
  const [catalogFocus, setCatalogFocusState] = useState<CatalogFocus>(() =>
    typeof window === "undefined"
      ? emptyCatalogFocus
      : parseLibraryFocus(new URLSearchParams(window.location.search)),
  );
  const [sort, setSortState] = useState<CatalogSort>(() =>
    typeof window === "undefined"
      ? "artist"
      : parseCatalogSort(new URLSearchParams(window.location.search)),
  );
  const catalogFilter: CatalogFilter = {
    ...toCatalogFilter(catalogFocus),
    ...(sort === "artist" ? {} : { sort }),
  };

  const catalog = useInfiniteQuery({
    queryKey: ["library", "catalog", "songs"],
    initialPageParam: 0,
    queryFn: ({ pageParam }) =>
      getLibraryCatalog(pageParam, PAGE_SIZE, "songs"),
    getNextPageParam: (lastPage, pages) => {
      const next = pages.length * PAGE_SIZE;
      return next < lastPage.totalSongs ? next : undefined;
    },
    enabled: catalogEnabled,
  });
  // Local files and streaming library albums in one catalog, with the
  // storage-location filters applied on the server.
  const albumCatalog = useInfiniteQuery({
    queryKey: ["library", "catalog-albums", catalogFilter],
    initialPageParam: 0,
    queryFn: ({ pageParam }) =>
      getCatalogAlbums(catalogFilter, { offset: pageParam, limit: PAGE_SIZE }),
    getNextPageParam: (lastPage, pages) => {
      const next = pages.length * PAGE_SIZE;
      return next < lastPage.total ? next : undefined;
    },
    placeholderData: keepPreviousData,
    enabled: !focusActive && view === "albums",
  });
  const artists = useInfiniteQuery({
    queryKey: ["library", "catalog", "artists"],
    initialPageParam: 0,
    queryFn: ({ pageParam }) => getLibraryCatalogArtists(pageParam, PAGE_SIZE),
    getNextPageParam: (lastPage, pages) =>
      lastPage.length === PAGE_SIZE ? pages.length * PAGE_SIZE : undefined,
    enabled: !focusActive && view === "artists",
  });
  const focused = useInfiniteQuery({
    queryKey: ["library", "focus", "page", focusQueryKey(focus)],
    initialPageParam: 0,
    queryFn: ({ pageParam }) => getFocusPage(focus, pageParam, PAGE_SIZE),
    getNextPageParam: (lastPage, pages) => {
      const next = pages.length * PAGE_SIZE;
      return next < lastPage.total ? next : undefined;
    },
    enabled: focusActive,
    placeholderData: keepPreviousData,
  });
  const playlists = useQuery({
    queryKey: ["playlists"],
    queryFn: getPlaylists,
    enabled: view === "playlists",
  });

  const catalogPages = catalog.data?.pages ?? [];
  const focusPages = focused.data?.pages ?? [];
  const sources = useLibrarySources("track");
  const streamingFilter =
    sources.source === "tidal" || sources.source === "qobuz"
      ? sources.source
      : null;
  const albumPages = albumCatalog.data?.pages ?? [];
  const albumFacets = albumPages[0]?.facets;
  const albumTotal = albumPages[0]?.total;
  const allAlbums: LibraryAlbumEntry[] = focusActive
    ? uniqueById(focusPages.flatMap((page) => page.albums)).map(
        focusedAlbumEntry,
      )
    : albumPages.flatMap((page) => page.albums).map(catalogAlbumEntry);
  const allSongs: FocusedSong[] = focusActive
    ? uniqueById(focusPages.flatMap((page) => page.songs)).map((song) => ({
        ...song,
        badge: audioBadge(song.audio),
      }))
    : uniqueById(catalogPages.flatMap((page) => page.songs));
  const allArtists = focusActive
    ? uniqueById(focusPages.flatMap((page) => page.artists))
    : uniqueById(artists.data?.pages.flat() ?? []);
  const allComposers = uniqueById(
    focusPages.flatMap((page) => page.composers ?? []),
  );
  const lastFocusPage = focusPages[focusPages.length - 1];
  const lastCatalogPage = catalogPages[catalogPages.length - 1];
  const countLabel = focusActive
    ? focusCountLabel(focus, lastFocusPage?.total, lastFocusPage?.libraryTotal)
    : view === "albums"
      ? focusCountLabel(focus, albumTotal, undefined)
      : view === "songs"
        ? focusCountLabel(focus, lastCatalogPage?.totalSongs, undefined)
        : null;

  const active = focusActive
    ? focused
    : view === "artists"
      ? artists
      : view === "albums"
        ? albumCatalog
        : catalog;
  const initialLoading = focusActive
    ? focused.isPending && !focused.data
    : (catalogEnabled && catalog.isPending && !catalog.data) ||
      (view === "albums" && albumCatalog.isPending && !albumCatalog.data) ||
      (view === "artists" && artists.isPending && !artists.data) ||
      (view === "playlists" && playlists.isPending);
  const activeError = focusActive
    ? focused.error
    : ((catalogEnabled ? catalog.error : null) ??
      (view === "albums" ? albumCatalog.error : null) ??
      (view === "artists" ? artists.error : null) ??
      (view === "playlists" ? playlists.error : null));

  const writeCatalogParams = (
    write: (params: URLSearchParams) => URLSearchParams,
  ) => {
    const params = write(new URLSearchParams(window.location.search));
    window.history.replaceState(
      window.history.state,
      "",
      `/library${params.size ? `?${params}` : ""}`,
    );
  };
  const setCatalogFocus = (next: CatalogFocus) => {
    setCatalogFocusState(next);
    sources.setSource(next.source);
    writeCatalogParams((params) => writeLibraryFocus(params, next));
  };
  const setSort = (next: CatalogSort) => {
    setSortState(next);
    writeCatalogParams((params) => writeCatalogSort(params, next));
  };

  const selectView = (nextView: LibraryView) => {
    const nextFocus = isBrowseView(nextView)
      ? withEntity(focus, nextView)
      : focus;
    setView(nextView);
    setFocusState(nextFocus);
    writeURL(nextView, nextFocus);
  };

  const setFocus = useCallback((next: FocusQuery) => {
    const nextView = next.entity as LibraryView;
    setView(nextView);
    setFocusState(next);
    writeURL(nextView, next);
  }, []);

  const playFocus = (shuffle: boolean) => playFocusQuery(focus, shuffle);

  useEffect(() => {
    const syncFromHistory = () => {
      const params = new URLSearchParams(window.location.search);
      const requested = params.get("view");
      const nextView = views.includes(requested as LibraryView)
        ? (requested as LibraryView)
        : "albums";
      setView(nextView);
      setFocusState(decodeFocus(params.get("focus"), entityForView(nextView)));
    };
    window.addEventListener("popstate", syncFromHistory);
    return () => window.removeEventListener("popstate", syncFromHistory);
  }, []);

  const clearFocus = () => setFocus(emptyFocus(focus.entity));

  return (
    <section className="mx-auto w-full max-w-266 space-y-7">
      <header className="flex items-end justify-between gap-5">
        <div>
          <h1 className="text-3xl font-semibold text-white">Library</h1>
          {browsing && countLabel && (
            <p
              aria-live="polite"
              className="mt-1 text-sm tabular-nums text-zinc-500"
            >
              {countLabel}
            </p>
          )}
        </div>
        {session?.role === "admin" && (
          <Link
            className="flex h-9 items-center gap-2 rounded-full border border-white/10 px-3.5 text-xs font-semibold text-zinc-300 transition-colors hover:border-white/25 hover:text-white"
            href="/library/tools"
          >
            <Wrench className="h-3.5 w-3.5" />
            Library tools
          </Link>
        )}
      </header>

      <div className="flex min-h-11 min-w-0 items-end border-b border-white/8">
        <div
          aria-label="Library views"
          className="flex min-w-0 flex-1 gap-1 overflow-x-auto scrollbar-none [&::-webkit-scrollbar]:hidden"
          role="tablist"
        >
          <LibraryTab
            active={view === "albums"}
            icon={<Disc3 />}
            label="Albums"
            onClick={() => selectView("albums")}
          />
          <LibraryTab
            active={view === "songs"}
            icon={<Music2 />}
            label="Songs"
            onClick={() => selectView("songs")}
          />
          <LibraryTab
            active={view === "artists"}
            icon={<UserRound />}
            label="Artists"
            onClick={() => selectView("artists")}
          />
          <LibraryTab
            active={view === "composers"}
            icon={<Feather />}
            label="Composers"
            onClick={() => selectView("composers")}
          />
          <LibraryTab
            active={view === "compositions"}
            icon={<ScrollText />}
            label="Compositions"
            onClick={() => selectView("compositions")}
          />
          <LibraryTab
            active={view === "playlists"}
            icon={<ListMusic />}
            label="Playlists"
            onClick={() => selectView("playlists")}
          />
          <LibraryTab
            active={view === "banned"}
            icon={<Ban />}
            label="Banned"
            onClick={() => selectView("banned")}
          />
        </div>
        {view === "playlists" && (
          <div className="mb-2 ml-2 flex shrink-0 gap-2">
            <Button asChild variant="ghost">
              <Link href="/imports">
                <Import /> Import
              </Link>
            </Button>
            <CreatePlaylistButton />
          </div>
        )}
      </div>

      {browsing && (
        <div className="space-y-4">
          <FocusBar
            canPlay={
              view === "albums" ||
              view === "songs" ||
              view === "compositions" ||
              (view === "artists" && focusActive)
            }
            onCreateSmartPlaylist={() => setSmartOpen(true)}
            onPlay={(shuffle) => void playFocus(shuffle)}
            onTogglePanel={() => setPanelOpen((open) => !open)}
            panelOpen={panelOpen}
            query={focus}
            setQuery={setFocus}
          />
          {panelOpen && <FocusPanel query={focus} setQuery={setFocus} />}
        </div>
      )}
      <SmartPlaylistDialog
        onOpenChange={setSmartOpen}
        open={smartOpen}
        query={focus}
      />

      {activeError && (
        <p className="rounded-lg border border-amber-300/20 bg-amber-300/6 px-4 py-3 text-sm text-amber-100">
          Some items could not be loaded. You can retry below.
        </p>
      )}

      {view === "songs" && !focusActive && sources.showFilters && (
        <SourceFilterChips
          available={sources.available}
          onChange={(next) =>
            setCatalogFocus({ ...catalogFocus, source: next })
          }
          source={sources.source}
        />
      )}
      {view === "albums" &&
        !focusActive &&
        (albumTotal !== undefined || albumFacets) && (
          <CatalogFocusBar
            available={sources.available}
            facets={albumFacets}
            focus={catalogFocus}
            onChange={setCatalogFocus}
            onSortChange={setSort}
            sort={sort}
            total={albumTotal}
          />
        )}

      {view === "albums" &&
        (!focusActive &&
        !initialLoading &&
        !allAlbums.length &&
        streamingFilter &&
        activeFocusCount(catalogFocus) === 1 ? (
          <StreamingLibraryEmpty kind="albums" provider={streamingFilter} />
        ) : !focusActive &&
          !initialLoading &&
          !allAlbums.length &&
          activeFocusCount(catalogFocus) > 0 ? (
          <LibraryMessage
            title="No albums match this focus"
            body="Turn off a filter, or choose Clear focus to see every album again."
          />
        ) : (
          <AlbumsView
            albums={allAlbums}
            focused={focusActive}
            hasMore={active.hasNextPage}
            loading={initialLoading}
            loadingMore={active.isFetchingNextPage}
            onClearFocus={clearFocus}
            onLoadMore={() => void active.fetchNextPage()}
          />
        ))}
      {view === "songs" &&
        (!focusActive && streamingFilter ? (
          sources.loading ? (
            <LibraryLoading compact />
          ) : sources.tracks.length ? (
            <StreamingTrackList tracks={sources.tracks} />
          ) : (
            <StreamingLibraryEmpty kind="tracks" provider={streamingFilter} />
          )
        ) : (
          <>
            <SongsView
              focused={focusActive}
              hasMore={active.hasNextPage}
              loading={initialLoading}
              loadingMore={active.isFetchingNextPage}
              onClearFocus={clearFocus}
              onLoadMore={() => void active.fetchNextPage()}
              songs={allSongs}
            />
            {!focusActive &&
              sources.source === "all" &&
              !catalog.hasNextPage && (
                <StreamingTrackList
                  title="Streaming tracks in your library"
                  tracks={sources.tracks}
                />
              )}
          </>
        ))}
      {view === "artists" && (
        <ArtistsView
          artists={allArtists}
          focused={focusActive}
          hasMore={active.hasNextPage}
          loading={initialLoading}
          loadingMore={active.isFetchingNextPage}
          onClearFocus={clearFocus}
          onLoadMore={() => void active.fetchNextPage()}
        />
      )}
      {view === "composers" && (
        <ComposersView
          composers={allComposers}
          focused={isFocused(focus)}
          hasMore={active.hasNextPage}
          loading={initialLoading}
          loadingMore={active.isFetchingNextPage}
          onClearFocus={clearFocus}
          onLoadMore={() => void active.fetchNextPage()}
        />
      )}
      {view === "compositions" && (
        <CompositionList
          emptyBody="Compositions appear when your files carry COMPOSER or WORK tags."
          focus={focus}
          onClear={clearFocus}
        />
      )}
      {view === "banned" && <BannedView />}
      {view === "playlists" && (
        <PlaylistsView
          loading={playlists.isPending}
          playlists={playlists.data}
        />
      )}
    </section>
  );
}

function FocusEmpty({ onClear }: { onClear: () => void }) {
  return (
    <LibraryMessage
      action="Clear focus"
      body="Nothing in your library matches every filter. Remove a filter or switch to Match any."
      onAction={onClear}
      title="No matches"
    />
  );
}

function LibraryTab({
  active,
  icon,
  label,
  onClick,
}: {
  active: boolean;
  icon?: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      aria-selected={active}
      className={`relative flex h-11 shrink-0 items-center gap-2 px-4 text-sm font-medium transition-colors [&_svg]:h-4 [&_svg]:w-4 ${
        active ? "text-white" : "text-zinc-500 hover:text-zinc-200"
      }`}
      onClick={onClick}
      role="tab"
      type="button"
    >
      {icon}
      {label}
      {active && (
        <span className="absolute inset-x-3 bottom-0 h-0.5 bg-white" />
      )}
    </button>
  );
}

function AlbumsView({
  albums,
  focused,
  hasMore,
  loading,
  loadingMore,
  onClearFocus,
  onLoadMore,
}: {
  albums: LibraryAlbumEntry[];
  focused: boolean;
  hasMore: boolean;
  loading: boolean;
  loadingMore: boolean;
  onClearFocus: () => void;
  onLoadMore: () => void;
}) {
  if (loading) {
    return <LibraryLoading compact />;
  }
  if (!albums.length && focused) {
    return <FocusEmpty onClear={onClearFocus} />;
  }
  if (!albums.length) {
    return (
      <LibraryMessage
        title="No albums yet"
        body="Add music to your indexed folder. New albums appear here automatically."
        href="/settings"
        action="Open settings"
      />
    );
  }
  return (
    <>
      <div className="grid grid-cols-2 gap-x-4 gap-y-10 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
        {albums.map((album) => (
          <AlbumCard
            key={album.key}
            album_id={album.id}
            album_name={album.name}
            album_cover={album.cover}
            artist_id={album.artistId}
            artist_name={album.artistName}
            first_release_date={album.year}
            formatLabel={album.formatLabel}
            source={album.source ?? undefined}
          />
        ))}
      </div>
      <InfiniteLoad
        hasMore={hasMore}
        loading={loadingMore}
        onLoadMore={onLoadMore}
      />
    </>
  );
}

function SongsView({
  focused,
  songs,
  hasMore,
  loading,
  loadingMore,
  onClearFocus,
  onLoadMore,
}: {
  focused: boolean;
  songs: FocusedSong[];
  hasMore: boolean;
  loading: boolean;
  loadingMore: boolean;
  onClearFocus: () => void;
  onLoadMore: () => void;
}) {
  if (loading) {
    return <LibraryLoading compact />;
  }
  if (!songs.length && focused) {
    return <FocusEmpty onClear={onClearFocus} />;
  }
  if (!songs.length) {
    return (
      <LibraryMessage
        title="No songs yet"
        body="Add supported audio files to your indexed folder. They appear automatically when copying finishes."
        href="/settings"
        action="Open settings"
      />
    );
  }
  return (
    <>
      <div className="overflow-hidden rounded-xl border border-white/8">
        {songs.map((song, index) => (
          <CatalogSongRow key={song.id} index={index} song={song} />
        ))}
      </div>
      <InfiniteLoad
        hasMore={hasMore}
        loading={loadingMore}
        onLoadMore={onLoadMore}
      />
    </>
  );
}

function CatalogSongRow({ index, song }: { index: number; song: FocusedSong }) {
  const playback = useZonePlayback();
  const active = playback.currentSongId === song.id;
  const artwork = song.coverPath
    ? `${getBaseURL()}/media/images/${encodeURIComponent(song.coverPath)}`
    : defaultCover;

  const play = () => {
    if (active) {
      void playback.togglePlayPause();
      return;
    }
    const artist = { id: song.artistId, name: song.artistName };
    const album = {
      id: song.albumId,
      name: song.albumName,
      cover_url: song.coverPath,
    };
    const track = {
      id: song.id,
      name: song.name,
      artist: song.artistName,
      path: song.path,
      duration: song.durationSeconds,
    };
    void playback.playQueue([{ song: track, artist, album }]);
  };

  return (
    <SongMenu
      album_id={song.albumId}
      album_name={song.albumName}
      album_cover={song.coverPath}
      artist_id={song.artistId}
      artist_name={song.artistName}
      song_id={song.id}
      song_name={song.name}
    >
      <div className="group grid grid-cols-[2rem_3rem_minmax(0,1fr)_auto] items-center gap-2 border-b border-white/6 px-3 py-2 last:border-0 hover:bg-white/[0.035] sm:grid-cols-[2rem_3rem_minmax(0,1fr)_minmax(8rem,0.6fr)_auto] sm:gap-3">
        <button
          aria-label={`${active && playback.isPlaying ? "Pause" : "Play"} ${song.name}`}
          className="flex h-8 w-8 items-center justify-center rounded-full text-zinc-500 hover:bg-white/10 hover:text-white"
          onClick={play}
          type="button"
        >
          {active && playback.isPlaying ? (
            <Pause className="h-4 w-4 fill-current text-white" />
          ) : (
            <>
              <span className="text-xs group-hover:hidden">{index + 1}</span>
              <Play className="hidden h-4 w-4 fill-current group-hover:block" />
            </>
          )}
        </button>
        <div className="relative h-10 w-10 overflow-hidden rounded-md bg-zinc-900">
          <Image
            alt=""
            className="object-cover"
            fill
            sizes="40px"
            src={artwork}
          />
        </div>
        <div className="min-w-0">
          <p
            className={`truncate text-sm font-medium ${active ? "text-white" : "text-zinc-200"}`}
          >
            <SongTitle title={song.name} />
            <BannedBadge
              className="ml-1.5 align-[-2px]"
              target={{ kind: "song", id: song.id }}
            />
          </p>
          <p className="truncate text-xs text-zinc-500 hover:text-white hover:underline sm:hidden">
            {isExplicitSongTitle(song.name) && <ExplicitBadge />}
            <ArtistCreditLinks credits={catalogSongArtistCredits(song)} />
          </p>
        </div>
        <p className="hidden truncate text-sm text-zinc-500 hover:text-white hover:underline sm:block">
          {isExplicitSongTitle(song.name) && <ExplicitBadge />}
          <ArtistCreditLinks credits={catalogSongArtistCredits(song)} />
        </p>
        <span className="flex items-center gap-3 pr-2 text-xs tabular-nums text-zinc-600">
          {song.badge && (
            <span className="hidden rounded border border-white/10 px-1.5 py-0.5 text-[10px] font-medium text-zinc-400 md:inline">
              {song.badge}
            </span>
          )}
          {formatDuration(song.durationSeconds)}
        </span>
      </div>
    </SongMenu>
  );
}

function ArtistsView({
  artists,
  focused,
  hasMore,
  loading,
  loadingMore,
  onClearFocus,
  onLoadMore,
}: {
  artists: LibraryCatalogArtist[];
  focused: boolean;
  hasMore: boolean;
  loading: boolean;
  loadingMore: boolean;
  onClearFocus: () => void;
  onLoadMore: () => void;
}) {
  if (loading) {
    return <LibraryLoading compact />;
  }
  if (!artists.length && focused) {
    return <FocusEmpty onClear={onClearFocus} />;
  }
  if (!artists.length) {
    return (
      <LibraryMessage
        title="No artists yet"
        body="Artists will appear after Parson indexes your collection."
      />
    );
  }
  return (
    <>
      <div className="overflow-hidden rounded-xl border border-white/8">
        {artists.map((artist) => (
          <ArtistRow artist={artist} key={artist.id} />
        ))}
      </div>
      <InfiniteLoad
        hasMore={hasMore}
        loading={loadingMore}
        onLoadMore={onLoadMore}
      />
    </>
  );
}

function ArtistRow({ artist }: { artist: LibraryCatalogArtist }) {
  const summary = artistCatalogSummary(artist);
  return (
    <ArtistMenu artistId={artist.id} artistName={artist.name}>
      <Link
        className="group flex min-w-0 items-center gap-4 border-b border-white/6 px-4 py-4 transition-colors last:border-0 hover:bg-white/[0.035] sm:px-5"
        href={`/artist?id=${artist.id}`}
      >
        <div className="min-w-0 flex-1">
          <h2 className="flex min-w-0 items-center gap-1.5 text-sm font-semibold text-zinc-100 group-hover:text-white">
            <span className="truncate">{artist.name}</span>
            <BannedBadge target={{ kind: "artist", id: artist.id }} />
          </h2>
          {summary && <p className="mt-1 text-xs text-zinc-500">{summary}</p>}
        </div>
        <ChevronRight className="h-4 w-4 shrink-0 text-zinc-600 transition-colors group-hover:text-white" />
      </Link>
    </ArtistMenu>
  );
}

function ComposersView({
  composers,
  focused,
  hasMore,
  loading,
  loadingMore,
  onClearFocus,
  onLoadMore,
}: {
  composers: FocusComposer[];
  focused: boolean;
  hasMore: boolean;
  loading: boolean;
  loadingMore: boolean;
  onClearFocus: () => void;
  onLoadMore: () => void;
}) {
  if (loading) {
    return <LibraryLoading compact />;
  }
  if (!composers.length && focused) {
    return <FocusEmpty onClear={onClearFocus} />;
  }
  if (!composers.length) {
    return (
      <LibraryMessage
        title="No composers yet"
        body="Composers appear when your files carry COMPOSER tags."
      />
    );
  }
  return (
    <>
      <div className="overflow-hidden rounded-xl border border-white/8">
        {composers.map((composer) => (
          <Link
            className="group flex min-w-0 items-center gap-4 border-b border-white/6 px-4 py-4 transition-colors last:border-0 hover:bg-white/[0.035] sm:px-5"
            href={`/composer?id=${encodeURIComponent(composer.id)}`}
            key={composer.id}
          >
            <div className="grid h-10 w-10 shrink-0 place-items-center rounded-full bg-white/[0.06] text-sm font-semibold text-zinc-300">
              {composerInitials(composer.name)}
            </div>
            <div className="min-w-0 flex-1">
              <h2 className="truncate text-sm font-semibold text-zinc-100 group-hover:text-white">
                {composer.name}
              </h2>
              <p className="mt-1 truncate text-xs text-zinc-500">
                {[composerFacts(composer), composerSummary(composer)]
                  .filter(Boolean)
                  .join(" · ")}
              </p>
            </div>
            <ChevronRight className="h-4 w-4 shrink-0 text-zinc-600 transition-colors group-hover:text-white" />
          </Link>
        ))}
      </div>
      <InfiniteLoad
        hasMore={hasMore}
        loading={loadingMore}
        onLoadMore={onLoadMore}
      />
    </>
  );
}

function PlaylistsView({
  loading,
  playlists,
}: {
  loading: boolean;
  playlists?: Awaited<ReturnType<typeof getPlaylists>>;
}) {
  const [createOpen, setCreateOpen] = useState(false);
  return (
    <>
      <ContextMenu>
        <ContextMenuTrigger asChild>
          <div className="min-h-64">
            {loading ? (
              <LibraryLoading compact />
            ) : (
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
                <LikedSongsPlaylistCard />
                {playlists?.map((playlist) => (
                  <PlaylistCard key={playlist.id} playlist={playlist} />
                ))}
                <SavedPlaylistCopyCards />
              </div>
            )}
          </div>
        </ContextMenuTrigger>
        <ContextMenuContent className="w-52">
          <ContextMenuItem onSelect={() => setCreateOpen(true)}>
            <Plus />
            Create playlist
          </ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>
      <CreatePlaylistDialog onOpenChange={setCreateOpen} open={createOpen} />
    </>
  );
}

function LikedSongsPlaylistCard() {
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <Link
          aria-label="Open Liked Songs"
          className="group flex min-w-0 items-center gap-4 overflow-hidden rounded-xl border border-white/8 bg-white/2.5 p-4 transition-colors hover:bg-white/6"
          href="/playlist?system=liked"
        >
          <div className="grid h-14 w-14 shrink-0 place-items-center rounded-lg bg-linear-to-br from-rose-500 to-fuchsia-800 text-white">
            <Heart className="h-6 w-6 fill-current" />
          </div>
          <h2 className="min-w-0 flex-1 truncate text-sm font-semibold text-zinc-100">
            Liked Songs
          </h2>
          <ChevronRight className="h-4 w-4 shrink-0 text-zinc-600 transition-colors group-hover:text-white" />
        </Link>
      </ContextMenuTrigger>
      <ContextMenuContent className="w-52">
        <ContextMenuItem asChild>
          <Link href="/playlist?system=liked">
            <Heart className="h-4 w-4" />
            Go to Liked Songs
          </Link>
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

function PlaylistCard({
  playlist,
}: {
  playlist: Awaited<ReturnType<typeof getPlaylists>>[number];
}) {
  return (
    <PlaylistMenu playlistId={playlist.id}>
      <Link
        aria-label={`Open ${playlist.name}`}
        className="group flex min-w-0 items-center gap-4 overflow-hidden rounded-xl border border-white/8 bg-white/2.5 p-4 transition-colors hover:bg-white/6 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-white/30"
        href={`/playlist?id=${playlist.id}`}
      >
        <div className="pointer-events-none relative z-10 h-14 w-14 shrink-0">
          <PlaylistCover
            className="h-14 w-14 rounded-lg"
            songs={playlist.cover_songs}
          />
        </div>
        <h2 className="pointer-events-none relative z-10 min-w-0 flex-1 truncate text-sm font-semibold text-zinc-100">
          {playlist.name}
        </h2>
        <ChevronRight className="h-4 w-4 shrink-0 text-zinc-600 transition-colors group-hover:text-white" />
      </Link>
    </PlaylistMenu>
  );
}

function CreatePlaylistButton() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button onClick={() => setOpen(true)}>
        <Plus /> New playlist
      </Button>
      <CreatePlaylistDialog onOpenChange={setOpen} open={open} />
    </>
  );
}
