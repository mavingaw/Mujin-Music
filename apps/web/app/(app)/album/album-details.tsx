"use client";

import ListenLaterButton from "@/features/library/listen-later/listen-later-button";
import TagButton from "@/features/library/tags/tag-button";

import getBaseURL from "@/lib/api/server-url";
import { getLibraryImageUrl } from "@/lib/images/image-url";
import {
  addAlbumToPlaylist as addAlbumTracksToPlaylist,
  albumHeroYear,
  getPlaylists,
  refreshEnrichment,
  isStreamingId,
  localAlbumFormat,
  releaseTypeLabel,
  type AlbumVersionSubject,
  type LibraryAlbum,
} from "@parson/music-sdk";
import StreamingAlbumDetails from "@/features/streaming/streaming-album";
import {
  AlbumVersions,
  BetterVersionNotice,
} from "@/features/streaming/album-versions";
import {
  PreferredPlayCaption,
  usePreferredAlbumPlay,
} from "@/features/streaming/preferred-play";
import type { Artist } from "@parson/music-sdk/types";
import Image from "next/image";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { useMemo, useState } from "react";
import {
  Download,
  Globe2,
  ListPlus,
  Loader2,
  MoreHorizontal,
  Pencil,
  SlidersHorizontal,
  Plus,
  RefreshCw,
  ScanSearch,
  UserRound,
} from "lucide-react";
import { IdentifyAlbumDialog } from "@/features/library/works/identify-album-dialog";
import { GenreChips } from "@/features/library/genres/genre-chips";
import { AlbumGenresDialog } from "@/features/library/genres/album-genres-dialog";
import { downloadAlbum } from "@/lib/downloads/download-album";
import { StartRadioButton } from "@/features/player/radio/radio-controls";
import { AlbumFormat } from "@/features/player/signal-path-popover";
import { PlayNowSplit } from "@/features/player/play-now-split";
import { useZonePlayback } from "@/features/player/zones/use-zone-playback";
import { useSession } from "@/features/account/session-provider";
import AlbumEditor, {
  type AlbumEditorTab,
} from "@/features/library/album-editor";
import { CreditsEditor } from "@/features/library/credits/credits-editor";
import { BanToggleButton } from "@/features/library/radio-bans";
import { useAlbum, useArtist } from "@/features/library/use-library-entity";
import AlbumCard from "@/features/library/album-card";
import { ShelfRow } from "@/features/library/home/home-sections";
import { generatedCover } from "@/features/library/artwork/generated-cover";
import { rgbString } from "@/features/player/artwork-palette";
import { useArtworkPalette } from "@/features/player/use-artwork-palette";
import { useOptionalAppearance } from "@/features/appearance/theme-provider";
import CreatePlaylistDialog from "@/features/library/create-playlist-dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
  DropdownMenuItem,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from "@/components/ui/dropdown-menu";
import { usePageTitle } from "@/components/app/title-metadata";
import { useFitText } from "@/features/library/use-fit-text";
import { AlbumTrackList } from "./album-track-list";
import { AlbumVersionsStrip } from "@/features/library/tools/album-versions-strip";
import { AlbumAboutSection } from "./album-about";
import { CreditsList } from "@/features/library/credits/credits-list";
import { tagCreditsAsAbout } from "@/features/library/credits/credit-roles";
import { useAlbumAbout } from "@/features/library/credits/use-editorial";
import type { AboutCredit, AlbumAbout } from "@parson/music-sdk/types";
import EntityPageState from "@/features/library/entity-page-state";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

function formatTotalDuration(duration: number) {
  const minutes = Math.max(1, Math.round(duration / 60));
  if (minutes < 60) {
    return `${minutes} min`;
  }

  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  return remainingMinutes
    ? `${hours} hr ${remainingMinutes} min`
    : `${hours} hr`;
}

function splitQualifiedTitle(title: string) {
  const parenthesis = title.indexOf("(");
  const bracket = title.indexOf("[");
  const candidates = [parenthesis, bracket].filter((index) => index > 0);
  const splitAt = candidates.length ? Math.min(...candidates) : -1;
  if (splitAt < 0) {
    return null;
  }
  return [title.slice(0, splitAt).trimEnd(), title.slice(splitAt)] as const;
}

function releaseYear(album: LibraryAlbum, about: AlbumAbout | undefined) {
  // The hero follows the original release, never the edition year.
  if (about?.sources) return albumHeroYear(about, album.first_release_date);
  const facts = about?.facts;
  // Older servers: a release matched by title alone may be a later
  // recording; the tags' year then wins.
  const value =
    (facts?.exact
      ? facts.first_release_date || album.first_release_date
      : album.first_release_date || facts?.first_release_date) || "";
  return /^\d{4}/.exec(value)?.[0] ?? null;
}

type AlbumDetailsProps = {
  devAlbum?: LibraryAlbum;
};

export default function AlbumDetails(props: AlbumDetailsProps = {}) {
  const id = useSearchParams()?.get("id");
  if (!props.devAlbum && isStreamingId(id)) {
    return <StreamingAlbumDetails id={id ?? ""} />;
  }
  return <LibraryAlbumDetails {...props} />;
}

function LibraryAlbumDetails({ devAlbum }: AlbumDetailsProps = {}) {
  const searchParams = useSearchParams();
  const id = searchParams?.get("id");
  const { session } = useSession();

  const queryClient = useQueryClient();
  const [actionsOpen, setActionsOpen] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [identifyOpen, setIdentifyOpen] = useState(false);
  const [genresOpen, setGenresOpen] = useState(false);
  const [editorTab, setEditorTab] = useState<AlbumEditorTab | null>(null);
  const [editingCredits, setEditingCredits] = useState(false);
  const playlists = useQuery({
    queryKey: ["playlists"],
    queryFn: getPlaylists,
    enabled: actionsOpen,
  });

  const albumQuery = useAlbum(id, devAlbum);
  const album = albumQuery.data ?? null;
  const aboutQuery = useAlbumAbout(devAlbum ? null : album?.id);
  const [tab, setTab] = useState<"tracks" | "credits">("tracks");
  const albumCredits = useMemo<AboutCredit[]>(() => {
    if (aboutQuery.data) return aboutQuery.data.credits;
    return (album?.credits ?? []).map((credit) => ({
      artist_id: credit.artist_id,
      name: credit.name,
      role: credit.role,
      instrument: credit.instrument,
      track_ids: credit.track_ids,
      sources: ["tags"],
    }));
  }, [aboutQuery.data, album?.credits]);
  const trackCredits = useMemo(() => {
    const byTrack = new Map<string, AboutCredit[]>();
    for (const song of album?.songs ?? []) {
      const merged = albumCredits.filter((credit) =>
        credit.track_ids.includes(song.id),
      );
      byTrack.set(
        song.id,
        merged.length || aboutQuery.data
          ? merged
          : tagCreditsAsAbout(song.id, song.credits),
      );
    }
    return byTrack;
  }, [aboutQuery.data, album?.songs, albumCredits]);
  const releaseCredits = useMemo(
    () => albumCredits.filter((credit) => credit.track_ids.length === 0),
    [albumCredits],
  );
  const artist: Artist | null = album?.artist_object ?? null;
  usePageTitle(
    album ? [album.name, artist?.name].filter(Boolean).join(" - ") : null,
  );
  const {
    fontSize: titleFontSize,
    textRef: titleRef,
    wrapped: titleWrapped,
    wrapperRef: titleWrapRef,
  } = useFitText(album?.name, 30, 76);

  const playback = useZonePlayback();
  const { isPlaying, currentSongId, currentAlbumId } = playback;

  const albumCoverURL = useMemo(
    () => getLibraryImageUrl(album?.cover_url, getBaseURL),
    [album?.cover_url],
  );
  const heroPalette = useArtworkPalette(albumCoverURL);
  // OLED Black and High Contrast keep the cover's colour out of the page.
  const backdrop = useOptionalAppearance()?.theme.backdrop ?? "artwork";
  const artGlow = backdrop === "artwork" || backdrop === "glass";
  const artistIconURL = useMemo(
    () => getLibraryImageUrl(artist?.icon_url, getBaseURL),
    [artist?.icon_url],
  );

  const totalDuration = useMemo(() => {
    return (album?.songs || []).reduce(
      (acc, song) => acc + (song.duration || 0),
      0,
    );
  }, [album?.songs]);
  const addAlbumToPlaylist = useMutation({
    mutationFn: async ({ id }: { id: number; name: string }) => {
      if (!album) {
        throw new Error("Album unavailable");
      }
      await addAlbumTracksToPlaylist(id, album.id);
    },
    onSuccess: async (_, playlist) => {
      toast.success(`Added album to ${playlist.name}`);
      await queryClient.invalidateQueries({
        queryKey: ["playlist", playlist.id],
        refetchType: "none",
      });
      await queryClient.invalidateQueries({
        queryKey: ["playlists"],
        refetchType: "none",
      });
    },
    onError: () => toast("Could not add this album."),
  });

  const saveAlbum = () => {
    if (!album) {
      return;
    }
    const count = downloadAlbum(album);
    if (count) {
      toast.success(`Downloading ${count} songs from ${album.name}`);
    }
  };

  const playTrack = (
    track: LibraryAlbum["songs"][number],
    tracks: LibraryAlbum["songs"] = album?.songs ?? [],
  ) => {
    if (!album || !artist) {
      return;
    }
    if (currentSongId === track.id) {
      void playback.togglePlayPause();
      return;
    }
    const index = tracks.findIndex((song) => song.id === track.id);
    void playback.playQueue(
      tracks.map((song) => ({ song, album, artist })),
      Math.max(index, 0),
    );
  };

  const playThisCopy = () => {
    const firstSong = album?.songs?.[0];
    if (!artist || !firstSong) {
      return;
    }
    playTrack(firstSong);
  };

  // The same album may also be on TIDAL or Qobuz; album-level Play starts
  // whichever copy the source preference picks.
  const subject = useMemo<AlbumVersionSubject | null>(
    () =>
      album && artist
        ? {
            id: String(album.id),
            title: album.name,
            artistName: artist.name,
            source: "local",
            format: localAlbumFormat(album.songs ?? []),
          }
        : null,
    [album, artist],
  );
  const preferred = usePreferredAlbumPlay(subject, playThisCopy);
  const playAlbum = () => void preferred.play();

  if (!id && !devAlbum) {
    return <EntityPageState kind="album" />;
  }
  if (albumQuery.isPending) {
    return <EntityPageState kind="album" loading />;
  }
  if (albumQuery.isError || !album || !artist) {
    return (
      <EntityPageState kind="album" onRetry={() => void albumQuery.refetch()} />
    );
  }

  const isAlbumPlaying = preferred.playing;
  const titleParts = titleWrapped ? splitQualifiedTitle(album.name) : null;

  const year = releaseYear(album, aboutQuery.data);
  // Only a named release type (an edit, MusicBrainz or the tags): never a
  // guess from the track count or running time.
  const typeLabel = releaseTypeLabel(aboutQuery.data);
  const heroCover =
    albumCoverURL ?? generatedCover(album.name, artist?.name ?? "");

  return (
    <div className="relative isolate text-mujin-text">
      <AlbumHeroBackdrop cover={heroCover} />
      <div className="relative mx-auto max-w-[1240px] px-5 pb-24 pt-8 sm:px-8 md:pt-12 lg:px-12">
        <div className="mb-9 grid w-full gap-7 md:grid-cols-[minmax(220px,340px)_minmax(0,1fr)] md:items-end lg:grid-cols-[minmax(260px,380px)_minmax(0,1fr)] lg:gap-12">
          <div
            className="relative aspect-square w-full max-w-[300px] shrink-0 overflow-hidden rounded-[calc(8px*var(--mj-radius-scale,1))] bg-mujin-surface-1 md:max-w-none"
            style={{
              boxShadow: artGlow
                ? `0 0 0 1px rgb(var(--mj-ink) / 0.08), 0 40px 90px -36px ${rgbString(heroPalette.glow, 0.6)}, 0 24px 50px -26px rgb(0 0 0 / 0.9)`
                : "0 0 0 1px var(--color-mujin-border), 0 24px 50px -26px rgb(0 0 0 / 0.9)",
            }}
          >
            <Image
              src={heroCover}
              alt={album.name}
              fill
              className="object-cover"
              sizes="(min-width: 1024px) 380px, (min-width: 768px) 340px, 300px"
              priority
            />
          </div>

          <div
            ref={titleWrapRef}
            className="flex w-full min-w-0 flex-1 flex-col"
          >
            <p className="mj-eyebrow mb-3">
              {[typeLabel, year].filter(Boolean).join(" · ")}
            </p>
            <h1
              ref={titleRef}
              className={`mj-display mb-4 text-mujin-text ${titleWrapped ? "text-balance whitespace-normal" : "whitespace-nowrap"}`}
              style={{ fontSize: titleFontSize }}
            >
              {titleParts ? (
                <>
                  <span className="block">{titleParts[0]}</span>
                  <span className="block text-mujin-text-secondary">
                    {titleParts[1]}
                  </span>
                </>
              ) : (
                album.name
              )}
            </h1>
            <div className="flex flex-wrap items-center gap-2.5">
              {artistIconURL && (
                <div className="relative h-7 w-7 overflow-hidden rounded-full">
                  <Image
                    src={artistIconURL}
                    alt={artist.name}
                    fill
                    className="object-cover"
                    sizes="28px"
                  />
                </div>
              )}
              <Link
                href={`/artist?id=${artist.id}`}
                className="mj-title cursor-pointer text-[1.25rem] text-mujin-text hover:underline md:text-[1.375rem]"
              >
                {artist.name}
              </Link>
            </div>
            <div className="mj-tnum mt-3 flex flex-wrap items-center gap-x-2 gap-y-1 text-[0.8125rem] text-mujin-text-secondary">
              {year && <span title="Original release">{year}</span>}
              {year && <span className="text-mujin-text-disabled">•</span>}
              <span>
                {album.songs.length}{" "}
                {album.songs.length === 1 ? "track" : "tracks"},{" "}
                {formatTotalDuration(totalDuration)}
              </span>
              <AlbumFormat audios={album.songs.map((song) => song.audio)} />
              {album.label && (
                <>
                  <span className="text-mujin-text-disabled">•</span>
                  <span>{album.label}</span>
                </>
              )}
            </div>
            <GenreChips
              className="mt-4"
              genres={aboutQuery.data?.genres}
              onEdit={
                session?.role === "admin" && aboutQuery.data
                  ? () => setGenresOpen(true)
                  : undefined
              }
            />
          </div>
        </div>

        <div className="mb-9 flex flex-wrap items-center gap-3">
          <PlayNowSplit
            busy={preferred.starting}
            onPlayNow={playAlbum}
            playing={isAlbumPlaying}
            target={{ kind: "album", id: album.id, name: album.name }}
          />
          <StartRadioButton
            label={`Start radio from ${album.name}`}
            seed={{ kind: "album", id: album.id }}
          />
          <ListenLaterButton
            entityId={album.id}
            entityType="album"
            name={album.name}
          />
          <TagButton
            entityIds={[album.id]}
            entityType="album"
            title={album.name}
          />
          <BanToggleButton
            target={{ kind: "album", id: album.id, name: album.name }}
          />
          <PreferredPlayCaption
            onPlayThisCopy={playThisCopy}
            play={preferred}
          />
          <DropdownMenu open={actionsOpen} onOpenChange={setActionsOpen}>
            <DropdownMenuTrigger asChild>
              <button
                aria-label="Album actions"
                className="grid h-10 w-10 place-items-center rounded-full border border-white/12 text-mujin-text-secondary transition-colors hover:border-white/25 hover:text-mujin-text"
                type="button"
              >
                <MoreHorizontal className="h-5 w-5" />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" className="w-56">
              {session?.role === "user" && (
                <DropdownMenuItem onSelect={saveAlbum}>
                  <Download className="h-4 w-4" />
                  Download album
                </DropdownMenuItem>
              )}
              <DropdownMenuItem asChild>
                <Link href={`/artist?id=${artist.id}`}>
                  <UserRound className="h-4 w-4" />
                  View artist
                </Link>
              </DropdownMenuItem>
              <DropdownMenuSub>
                <DropdownMenuSubTrigger>
                  <ListPlus className="h-4 w-4" />
                  Add to playlist
                </DropdownMenuSubTrigger>
                <DropdownMenuSubContent className="w-52">
                  {playlists.isPending && (
                    <DropdownMenuItem disabled>
                      <Loader2 className="h-4 w-4 animate-spin" />
                      Loading playlists…
                    </DropdownMenuItem>
                  )}
                  {playlists.isError && (
                    <DropdownMenuItem onSelect={() => void playlists.refetch()}>
                      <RefreshCw className="h-4 w-4" />
                      Try loading again
                    </DropdownMenuItem>
                  )}
                  {playlists.data?.map((playlist) => (
                    <DropdownMenuItem
                      disabled={addAlbumToPlaylist.isPending}
                      key={playlist.id}
                      onSelect={() =>
                        addAlbumToPlaylist.mutate({
                          id: playlist.id,
                          name: playlist.name,
                        })
                      }
                    >
                      <ListPlus className="h-4 w-4" />
                      <span className="truncate">{playlist.name}</span>
                    </DropdownMenuItem>
                  ))}
                  {playlists.isSuccess && (
                    <DropdownMenuItem onSelect={() => setCreateOpen(true)}>
                      <Plus className="h-4 w-4" />
                      New playlist
                    </DropdownMenuItem>
                  )}
                </DropdownMenuSubContent>
              </DropdownMenuSub>
              {session?.role === "admin" &&
                aboutQuery.data?.enrichment.enabled && (
                  <DropdownMenuItem
                    onSelect={() =>
                      void refreshEnrichment("album", album.id).then(
                        () =>
                          toast.success(
                            "Mujin Music will look this album up again shortly.",
                          ),
                        () => toast("Could not queue the lookup."),
                      )
                    }
                  >
                    <Globe2 className="h-4 w-4" />
                    Look up online metadata again
                  </DropdownMenuItem>
                )}
              {session?.role === "admin" && (
                <DropdownMenuItem onSelect={() => setIdentifyOpen(true)}>
                  <ScanSearch className="h-4 w-4" />
                  Identify album…
                </DropdownMenuItem>
              )}
              {session?.role === "admin" && (
                <DropdownMenuItem onSelect={() => setEditorTab("details")}>
                  <Pencil className="h-4 w-4" />
                  Edit album metadata
                </DropdownMenuItem>
              )}
              {session?.role === "admin" && aboutQuery.data?.sources && (
                <DropdownMenuItem onSelect={() => setEditorTab("sources")}>
                  <SlidersHorizontal className="h-4 w-4" />
                  Metadata preference…
                </DropdownMenuItem>
              )}
            </DropdownMenuContent>
          </DropdownMenu>
          {session?.role === "admin" && (
            <AlbumEditor
              albumId={album.id}
              initialTab={editorTab ?? "details"}
              onEditCredits={() => {
                setTab("credits");
                setEditingCredits(true);
              }}
              onOpenChange={(next) => !next && setEditorTab(null)}
              open={editorTab !== null}
              trigger="none"
            />
          )}
          <CreatePlaylistDialog
            initialAlbumId={album?.id}
            onOpenChange={setCreateOpen}
            open={createOpen}
          />
          {session?.role === "admin" && aboutQuery.data && (
            <AlbumGenresDialog
              albumId={album.id}
              albumName={album.name}
              edited={Boolean(aboutQuery.data.genres_edited)}
              genres={aboutQuery.data.genres ?? []}
              onOpenChange={setGenresOpen}
              open={genresOpen}
            />
          )}
          {session?.role === "admin" && (
            <IdentifyAlbumDialog
              albumId={album.id}
              albumName={album.name}
              onOpenChange={setIdentifyOpen}
              open={identifyOpen}
            />
          )}
        </div>

        {subject && <BetterVersionNotice subject={subject} />}

        <AlbumAboutSection
          about={aboutQuery.data}
          album={album}
          isAdmin={session?.role === "admin"}
          onEditSources={
            session?.role === "admin" && aboutQuery.data?.sources
              ? () => setEditorTab("sources")
              : undefined
          }
          onIdentify={() => setIdentifyOpen(true)}
        />

        <div
          aria-label="Album sections"
          className="mb-5 flex gap-8 border-b border-white/[0.08]"
          role="tablist"
        >
          {(
            [
              ["tracks", "Tracks"],
              ["credits", "Credits"],
            ] as const
          ).map(([key, label]) => (
            <button
              aria-controls={`album-${key}`}
              aria-selected={tab === key}
              className={`-mb-px border-b-2 pb-3 text-[0.75rem] font-semibold uppercase tracking-[0.14em] transition-colors ${
                tab === key
                  ? "border-mujin-copper text-mujin-text"
                  : "border-transparent text-mujin-text-tertiary hover:text-mujin-text"
              }`}
              id={`album-tab-${key}`}
              key={key}
              onClick={() => setTab(key)}
              role="tab"
              type="button"
            >
              {label}
              {key === "credits" && albumCredits.length > 0 && (
                <span className="ml-2 text-xs font-medium text-zinc-500">
                  {new Set(albumCredits.map((credit) => credit.name)).size}
                </span>
              )}
            </button>
          ))}
        </div>

        {tab === "tracks" ? (
          <div
            aria-labelledby="album-tab-tracks"
            id="album-tracks"
            role="tabpanel"
          >
            <AlbumTrackList
              activeSongId={currentSongId ?? undefined}
              album={album}
              artistName={artist.name}
              isPlaying={isPlaying}
              onPlay={playTrack}
              trackCredits={trackCredits}
              works={
                aboutQuery.data ? (aboutQuery.data.works ?? []) : undefined
              }
            />
          </div>
        ) : (
          <div
            aria-labelledby="album-tab-credits"
            className="space-y-10 pb-10"
            id="album-credits"
            role="tabpanel"
          >
            {session?.role === "admin" &&
              aboutQuery.data?.sources &&
              !editingCredits && (
                <div className="-mt-2 flex flex-wrap items-center justify-between gap-3">
                  <p className="text-xs text-zinc-500">
                    {aboutQuery.data.sources.credits_edited
                      ? "These credits were edited by hand and are frozen against lookups."
                      : "Credits from your tags, plus MusicBrainz where the album prefers it."}
                  </p>
                  <button
                    className="flex items-center gap-1.5 rounded-full border border-white/12 px-3 py-1.5 text-xs font-semibold text-zinc-300 transition-colors hover:border-white/25 hover:text-white"
                    onClick={() => setEditingCredits(true)}
                    type="button"
                  >
                    <Pencil className="h-3.5 w-3.5" />
                    Edit credits
                  </button>
                </div>
              )}
            {editingCredits && aboutQuery.data ? (
              <CreditsEditor
                albumId={album.id}
                credits={albumCredits}
                edited={Boolean(aboutQuery.data.sources?.credits_edited)}
                onDone={() => setEditingCredits(false)}
                tracks={album.songs}
              />
            ) : (
              <CreditsList
                credits={albumCredits}
                emptyMessage={
                  aboutQuery.data?.enrichment.enabled
                    ? "No credits were found in this album's tags or on MusicBrainz."
                    : "No credits in this album's tags. Composer, producer and performer tags appear here, and online metadata can add MusicBrainz credits."
                }
                tracks={album.songs}
              />
            )}
            {releaseCredits.length > 0 &&
              releaseCredits.length < albumCredits.length && (
                <p className="text-xs text-zinc-500">
                  Credits without track numbers apply to the whole release.
                </p>
              )}
          </div>
        )}

        {subject && <AlbumVersions subject={subject} />}
        <AlbumVersionsStrip albumId={album.id} />
        <MoreByArtist albumId={album.id} artist={artist} />
      </div>
    </div>
  );
}

/** The cover, blurred and faded into the canvas behind the album header. */
function AlbumHeroBackdrop({ cover }: { cover: string }) {
  return (
    <div
      aria-hidden="true"
      className="pointer-events-none absolute inset-x-0 top-0 -z-10 h-[620px] overflow-hidden [mask-image:linear-gradient(to_bottom,black_75%,transparent)]"
    >
      <div
        className="mj-art-backdrop opacity-50!"
        style={{ backgroundImage: `url("${cover}")` }}
      />
      <div className="absolute inset-0 bg-linear-to-b from-mujin-base/30 via-mujin-base/70 to-mujin-base" />
    </div>
  );
}

/** Roon's "More by" strip: the artist's other albums in the library. */
function MoreByArtist({
  albumId,
  artist,
}: {
  albumId: string;
  artist: Artist;
}) {
  const artistQuery = useArtist(artist.id);
  const albums = (artistQuery.data?.albums ?? []).filter(
    (other) => other.id !== albumId && (other.songs?.length ?? 1) > 0,
  );
  if (!albums.length) return null;
  return (
    <section className="mt-16">
      <ShelfRow
        action={{
          href: `/artist?id=${encodeURIComponent(artist.id)}`,
          label: "Artist",
        }}
        eyebrow={artist.name}
        title={`More by ${artist.name}`}
      >
        {albums.slice(0, 12).map((other) => (
          <div className="min-w-0 snap-start" key={other.id}>
            <AlbumCard
              album_cover={other.cover_url}
              album_id={other.id}
              album_name={other.name}
              artist_id={artist.id}
              artist_name={artist.name}
              first_release_date={other.first_release_date}
              showYear
            />
          </div>
        ))}
      </ShelfRow>
    </section>
  );
}
