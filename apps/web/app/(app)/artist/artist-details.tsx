"use client";

import { AlbumArtFallback, getLibraryImageUrl } from "@/lib/images/image-url";
import getBaseURL from "@/lib/api/server-url";
import { Album, Artist } from "@parson/music-sdk/types";
import {
  getAlbumInfos,
  getApiBaseURL,
  isApiError,
  isStreamingId,
  type LibraryAlbum,
} from "@parson/music-sdk";
import StreamingArtistDetails from "@/features/streaming/streaming-artist";
import { useArtist } from "@/features/library/use-library-entity";
import { SongTitle, ExplicitBadge } from "@/features/library/song-title";
import { isExplicitSongTitle } from "@/features/library/song-presentation";
import Image from "next/image";
import { PlayNowSplit } from "@/features/player/play-now-split";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useEffect, useMemo } from "react";
import { useSession } from "@/features/account/session-provider";
import {
  useArtistAbout,
  useArtistCredits,
  useArtistDiscography,
  useArtistRelated,
} from "@/features/library/credits/use-editorial";
import { ArtistDiscography } from "./artist-discography";
import { useFocusSelection } from "@/features/library/focus/use-focus-selection";
import {
  FOCUS_GROUPS,
  focusSelectionCount,
  type CatalogueScope,
  type FocusSelection,
  type WorkFilter,
} from "@parson/music-sdk";
import { ArtistCreditsPanel } from "./artist-credits";
import { ArtistCompositionsPanel } from "./artist-compositions";
import { useArtistWorks } from "@/features/library/works/use-works";
import { ArtistOverview } from "./artist-overview";
import { ArtistRelated } from "./artist-related";
import { PersonAvatar } from "@/features/library/credits/person-avatar";
import { lifeSpanFacts } from "@/features/library/credits/credit-roles";
import { GenreChips } from "@/features/library/genres/genre-chips";
import { useQuery } from "@tanstack/react-query";
import { getSimilarArtists } from "@parson/music-sdk";
import { useZonePlayback } from "@/features/player/zones/use-zone-playback";
import { ArtistCircle, ShelfRow } from "@/features/library/home/home-sections";
import { usePageTitle } from "@/components/app/title-metadata";
import EntityPageState from "@/features/library/entity-page-state";
import AlbumMenu from "@/features/library/album-menu";
import { StartRadioButton } from "@/features/player/radio/radio-controls";
import ListenLaterButton from "@/features/library/listen-later/listen-later-button";
import TagButton from "@/features/library/tags/tag-button";
import { BannedBadge, BanToggleButton } from "@/features/library/radio-bans";
import { songCreditsArtist } from "@/features/library/artist-appearance";
import { artistKicker } from "@/features/library/artist-roles";

type ArtistDetailsProps = {
  devArtist?: Artist;
  devAlbums?: Album[];
};

type ArtistTab = "overview" | "discography" | "compositions" | "credits";

const sortAlbumsByReleaseDate = (albums: Album[]) =>
  [...albums].sort((left, right) => {
    const leftTime = Date.parse(left.first_release_date || "");
    const rightTime = Date.parse(right.first_release_date || "");
    const safeLeft = Number.isFinite(leftTime) ? leftTime : -Infinity;
    const safeRight = Number.isFinite(rightTime) ? rightTime : -Infinity;
    return safeRight - safeLeft;
  });

export default function ArtistDetails(props: ArtistDetailsProps = {}) {
  const id = useSearchParams()?.get("id");
  if (!props.devArtist && isStreamingId(id)) {
    return <StreamingArtistDetails id={id ?? ""} />;
  }
  return <LibraryArtistDetails {...props} />;
}

function LibraryArtistDetails({
  devArtist,
  devAlbums,
}: ArtistDetailsProps = {}) {
  const searchParams = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const { session } = useSession();
  const playback = useZonePlayback();
  const id = searchParams?.get("id");
  const requestedTab = searchParams?.get("tab") as ArtistTab | null;
  const roleFilter = searchParams?.get("role") ?? null;

  const [urlSelection, setSelection] = useFocusSelection({
    tab: "discography",
  });
  const onDiscography = requestedTab === "discography" || !requestedTab;
  // Focus facets live in the URL only on the Discography tab (the
  // Compositions tab has its own form and period filters).
  const selection: FocusSelection = onDiscography ? urlSelection : {};
  const artistQuery = useArtist(id, devArtist);
  const artistMissing =
    artistQuery.isError &&
    isApiError(artistQuery.error) &&
    artistQuery.error.response?.status === 404;
  const artist = artistQuery.data ?? null;
  const aboutQuery = useArtistAbout(devArtist ? null : id);
  const about = aboutQuery.data;
  // A person opened under another of their ids (a second spelling, their
  // MusicBrainz id) moves to the one page that has everything.
  const canonicalId =
    about && id && about.artist_id !== id && about.requested_id === id
      ? about.artist_id
      : null;
  useEffect(() => {
    if (!canonicalId) return;
    const params = new URLSearchParams(searchParams?.toString() ?? "");
    params.set("id", canonicalId);
    router.replace(`${pathname}?${params.toString()}`, { scroll: false });
  }, [canonicalId, pathname, router, searchParams]);
  const discographyQuery = useArtistDiscography(
    devArtist || canonicalId ? null : id,
    selection,
  );
  const discography = discographyQuery.data;
  const focusActive = focusSelectionCount(selection) > 0;
  const hasRoles = Boolean(about?.roles.length);
  const creditsQuery = useArtistCredits(
    devArtist ? null : id,
    hasRoles || artistMissing,
  );
  const isComposer = Boolean(
    about?.roles.some((role) => role.role === "composer"),
  );
  const worksQuery = useArtistWorks(devArtist ? null : id, isComposer);
  const relatedQuery = useArtistRelated(devArtist ? null : id);
  const hasCompositions = Boolean(worksQuery.data?.works.length);
  const workFilter: WorkFilter = {
    form: searchParams?.get("form"),
    period: searchParams?.get("period"),
    key: searchParams?.get("key"),
    scope: searchParams?.get("scope") as CatalogueScope | null,
  };
  usePageTitle(artist?.name ?? about?.name ?? creditsQuery.data?.name);
  const albums = devAlbums ?? artist?.albums ?? [];
  const featuredAlbumIds = artist?.featured_on_album_ids ?? [];
  const featuredAlbumsQuery = useQuery({
    queryKey: ["artist-featured-albums", artist?.id, featuredAlbumIds],
    queryFn: () =>
      getAlbumInfos(featuredAlbumIds, false) as Promise<
        Record<string, LibraryAlbum>
      >,
    enabled: featuredAlbumIds.length > 0,
  });

  const artistIconURL = useMemo(
    () => getLibraryImageUrl(artist?.icon_url, getBaseURL),
    [artist?.icon_url],
  );

  const discographySections = useMemo(() => {
    if (artist?.discography?.length) {
      return artist.discography.map((section) => ({
        ...section,
        albums: sortAlbumsByReleaseDate(section.albums),
      }));
    }

    return albums.length
      ? [
          {
            key: "albums",
            title: "Albums",
            albums: sortAlbumsByReleaseDate(albums),
          },
        ]
      : [];
  }, [albums, artist?.discography]);
  const featuredAppearances = useMemo(
    () =>
      featuredAlbumIds.flatMap((albumId) => {
        const album = featuredAlbumsQuery.data?.[albumId];
        if (!album || !artist) {
          return [];
        }
        const featuredTrack = album.songs.find((song) =>
          songCreditsArtist(song, artist),
        );
        return featuredTrack ? [{ album, track: featuredTrack }] : [];
      }),
    [artist, featuredAlbumIds, featuredAlbumsQuery.data],
  );
  const hasDiscography =
    discographySections.length > 0 ||
    featuredAppearances.length > 0 ||
    Boolean(discography?.focus.total);
  const tabs: { key: ArtistTab; label: string }[] = [
    about ? { key: "overview" as const, label: "Overview" } : null,
    hasDiscography || artist
      ? { key: "discography" as const, label: "Discography" }
      : null,
    hasCompositions
      ? { key: "compositions" as const, label: "Compositions" }
      : null,
    hasRoles || artistMissing
      ? { key: "credits" as const, label: "Credits" }
      : null,
  ].filter((tab): tab is { key: ArtistTab; label: string } => Boolean(tab));
  const defaultTab: ArtistTab = focusActive
    ? "discography"
    : about?.biography
      ? "overview"
      : hasCompositions && !discographySections.length
        ? "compositions"
        : hasDiscography
          ? "discography"
          : hasRoles || artistMissing
            ? "credits"
            : about
              ? "overview"
              : "discography";
  const activeTab =
    tabs.find((tab) => tab.key === requestedTab)?.key ?? defaultTab;
  const navigate = (tab: ArtistTab, role: string | null = null) => {
    const params = new URLSearchParams(searchParams?.toString() ?? "");
    params.set("tab", tab);
    if (role) params.set("role", role);
    else params.delete("role");
    for (const facet of ["form", "period", "key", "scope", ...FOCUS_GROUPS]) {
      params.delete(facet);
    }
    router.replace(`${pathname}?${params.toString()}`, { scroll: false });
  };

  if (!id && !devArtist) {
    return <EntityPageState kind="artist" />;
  }
  if (
    artistQuery.isPending ||
    (artistMissing && creditsQuery.isPending) ||
    (aboutQuery.isPending && aboutQuery.fetchStatus === "fetching")
  ) {
    return <EntityPageState kind="artist" loading />;
  }
  const name = artist?.name ?? creditsQuery.data?.name ?? about?.name;
  if (!name) {
    return (
      <EntityPageState
        kind="artist"
        onRetry={() => void artistQuery.refetch()}
      />
    );
  }
  const editorialImageURL = about?.image
    ? `${getApiBaseURL()}${about.image.url}`
    : null;
  // The library's artist picture is usually one of their covers; only a
  // picture of its own (an artist.jpg) counts as a portrait.
  const iconIsPortrait = Boolean(
    artist?.icon_url &&
    !albums.some((album) => album.cover_url === artist.icon_url),
  );
  const heroImageURL =
    editorialImageURL ?? (iconIsPortrait ? artistIconURL : null);
  const subtitle = [
    about?.facts?.kind,
    about?.facts?.area,
    about?.facts?.disambiguation,
  ]
    .filter(Boolean)
    .join(" · ");

  const lifeSpan = lifeSpanFacts(about?.facts)
    .map((fact) => `${fact.label} ${fact.value}`)
    .join(" · ");
  const kicker = artistKicker({
    ownsAlbums: albums.length > 0,
    isComposer,
    inLibrary: Boolean(artist),
    kind: about?.facts?.kind,
  });

  const mosaicCovers = albums
    .map((album) => getLibraryImageUrl(album.cover_url, getBaseURL))
    .filter((url): url is string => Boolean(url));
  const playArtist = () => {
    if (!artist) return;
    const ordered = [...albums].sort((left, right) =>
      (left.first_release_date || "").localeCompare(
        right.first_release_date || "",
      ),
    );
    const items = ordered.flatMap((album) =>
      (album.songs ?? []).map((song) => ({ song, album, artist })),
    );
    if (items.length) void playback.playQueue(items);
  };

  return (
    <div className="relative isolate text-mujin-text">
      <header
        aria-label={`${name} header`}
        className="relative isolate mb-10 flex min-h-[360px] items-end overflow-hidden border-b border-white/[0.06] md:min-h-[440px]"
      >
        {heroImageURL ? (
          <ArtistPhotoHero src={heroImageURL} />
        ) : mosaicCovers.length > 0 ? (
          <ArtistMosaic covers={mosaicCovers} />
        ) : (
          <div
            aria-hidden
            className="absolute inset-0 -z-10 bg-[radial-gradient(90%_120%_at_15%_0%,rgba(217,139,95,0.16),transparent_60%),linear-gradient(to_bottom,var(--color-mujin-surface-1),var(--color-mujin-base))]"
          />
        )}
        <div className="mx-auto flex w-full max-w-[1240px] flex-col gap-6 px-5 pb-9 pt-24 sm:flex-row sm:items-end sm:px-8 lg:px-12">
          {heroImageURL ? null : mosaicCovers.length === 0 ? (
            <figure className="shrink-0">
              <PersonAvatar
                className="shadow-2xl shadow-black/60"
                name={name}
                size={150}
              />
            </figure>
          ) : null}
          <div className="min-w-0 pb-1">
            <p className="mj-eyebrow mb-3">{kicker}</p>
            <h1 className="mj-display min-w-0 wrap-break-word text-[48px] text-mujin-text sm:text-[72px] lg:text-[88px]">
              {name}
            </h1>
            {(subtitle || lifeSpan) && (
              <p className="mt-3 text-[14px] text-mujin-text-secondary">
                {[subtitle, lifeSpan].filter(Boolean).join(" · ")}
              </p>
            )}
            {artist && (
              <p className="mj-tnum mt-2 text-[13px] text-mujin-text-tertiary">
                {albums.length} {albums.length === 1 ? "album" : "albums"} in
                your library
                {artistYears(albums) ? ` · ${artistYears(albums)}` : ""}
              </p>
            )}
            {about?.credited_as?.length ? (
              <p className="mt-2 text-sm text-mujin-text-tertiary">
                Also credited as{" "}
                <span className="text-mujin-text-secondary">
                  {about.credited_as.join(", ")}
                </span>
              </p>
            ) : null}
            <GenreChips className="mt-4" genres={about?.genres} />
            {artist && (
              <div className="mt-6 flex flex-wrap items-center gap-3">
                {albums.some((album) => album.songs?.length) && (
                  <PlayNowSplit
                    onPlayNow={playArtist}
                    target={{
                      kind: "artist",
                      id: artist.id,
                      name: artist.name,
                    }}
                  />
                )}
                <StartRadioButton
                  label={`Start ${artist.name} radio`}
                  seed={{ kind: "artist", id: artist.id }}
                />
                <ListenLaterButton
                  entityId={artist.id}
                  entityType="artist"
                  name={artist.name}
                />
                <TagButton
                  entityIds={[artist.id]}
                  entityType="artist"
                  title={artist.name}
                />
                <BanToggleButton
                  target={{ kind: "artist", id: artist.id, name: artist.name }}
                />
              </div>
            )}
            {heroImageURL === editorialImageURL && about?.image && (
              <p className="mt-3 text-xs text-mujin-text-tertiary">
                Photo:{" "}
                <a
                  className="hover:text-mujin-text hover:underline"
                  href={about.image.page_url}
                  rel="noreferrer"
                  target="_blank"
                >
                  {about.image.author}
                </a>
                ,{" "}
                {about.image.license_url ? (
                  <a
                    className="hover:text-mujin-text hover:underline"
                    href={about.image.license_url}
                    rel="noreferrer license"
                    target="_blank"
                  >
                    {about.image.license}
                  </a>
                ) : (
                  about.image.license
                )}
                , via Wikimedia Commons
              </p>
            )}
          </div>
        </div>
      </header>

      <div className="relative z-10 mx-auto max-w-[1240px] px-5 pb-24 sm:px-8 lg:px-12">
        {tabs.length > 1 && (
          <div
            aria-label="Artist sections"
            className="mb-9 flex gap-6 overflow-x-auto shadow-[inset_0_-1px_0_rgba(242,238,230,0.08)] [scrollbar-width:none] sm:gap-8"
            role="tablist"
          >
            {tabs.map((tab) => (
              <button
                aria-selected={activeTab === tab.key}
                className={`shrink-0 whitespace-nowrap border-b-2 pb-3 text-[12px] font-semibold uppercase tracking-[0.14em] transition-colors ${
                  activeTab === tab.key
                    ? "border-mujin-copper text-mujin-text"
                    : "border-transparent text-mujin-text-tertiary hover:text-mujin-text"
                }`}
                key={tab.key}
                onClick={() => navigate(tab.key)}
                role="tab"
                type="button"
              >
                {tab.label}
              </button>
            ))}
          </div>
        )}

        {activeTab === "overview" && about && (
          <ArtistOverview
            about={about}
            isAdmin={session?.role === "admin"}
            onShowCompositions={() => navigate("compositions")}
            onShowCredits={(role) => navigate("credits", role)}
            onShowDiscography={() => navigate("discography")}
            ownWork={{
              albums: albums.length,
              tracks: albums.reduce(
                (total, album) => total + (album.songs?.length ?? 0),
                0,
              ),
            }}
            works={worksQuery.data}
          />
        )}

        {activeTab === "overview" && about && relatedQuery.data && (
          <div className="mt-14">
            <ArtistRelated name={about.name} relations={relatedQuery.data} />
          </div>
        )}

        {activeTab === "compositions" && worksQuery.data && (
          <ArtistCompositionsPanel
            filter={workFilter}
            onFilterChange={(filter) => {
              const params = new URLSearchParams(
                searchParams?.toString() ?? "",
              );
              params.set("tab", "compositions");
              for (const facet of ["form", "period", "key", "scope"] as const) {
                const value = filter[facet];
                if (value) params.set(facet, value);
                else params.delete(facet);
              }
              router.replace(`${pathname}?${params.toString()}`, {
                scroll: false,
              });
            }}
            works={worksQuery.data}
          />
        )}

        {activeTab === "credits" &&
          (creditsQuery.data ? (
            <ArtistCreditsPanel
              credits={creditsQuery.data}
              filter={roleFilter}
              onFilterChange={(role) => navigate("credits", role)}
            />
          ) : (
            <p className="text-sm text-zinc-500">
              {creditsQuery.isError
                ? "Credits could not be loaded."
                : "Loading credits…"}
            </p>
          ))}

        {activeTab === "discography" && discography && (
          <div className="space-y-12">
            <ArtistDiscography
              discography={discography}
              loading={discographyQuery.isFetching}
              onSelectionChange={setSelection}
              selection={selection}
            />
            {!focusActive && featuredAppearances.length > 0 && (
              <FeaturedAppearances appearances={featuredAppearances} />
            )}
          </div>
        )}

        {activeTab === "discography" && !discography && artist && (
          <div className="space-y-12">
            {discographySections.length > 0 && (
              <div className="space-y-12">
                {discographySections.map((section) => (
                  <section key={section.key}>
                    <h3 className="mb-5 text-xl font-semibold text-white">
                      {section.title}
                    </h3>
                    <div className="grid grid-cols-2 gap-5 sm:grid-cols-3 lg:grid-cols-4">
                      {section.albums.map((album) => (
                        <AlbumMenu
                          album_id={album.id}
                          album_name={album.name}
                          artist_id={artist?.id ?? ""}
                          key={album.id}
                          showArtist={false}
                        >
                          <Link
                            href={`/album?id=${album.id}`}
                            className="group min-w-0 max-w-47.5 cursor-pointer"
                          >
                            <div className="relative aspect-square w-full overflow-hidden rounded-lg border border-white/10 bg-white/4">
                              {getLibraryImageUrl(
                                album.cover_url,
                                getBaseURL,
                              ) ? (
                                <Image
                                  src={
                                    getLibraryImageUrl(
                                      album.cover_url,
                                      getBaseURL,
                                    ) ?? ""
                                  }
                                  alt={album.name}
                                  fill
                                  className="object-cover transition-transform duration-500 group-hover:scale-105"
                                  sizes="220px"
                                />
                              ) : (
                                <AlbumArtFallback />
                              )}
                              <div className="absolute inset-0 bg-black/20 opacity-0 transition-opacity group-hover:opacity-100" />
                              <BannedBadge
                                className="absolute bottom-2 left-2 rounded-full bg-black/75 p-1"
                                target={{ kind: "album", id: album.id }}
                              />
                            </div>
                            <div className="mt-3 min-w-0">
                              <h4
                                className="line-clamp-2 text-sm font-semibold leading-5 text-zinc-100"
                                title={album.name}
                              >
                                {album.name}
                              </h4>
                            </div>
                          </Link>
                        </AlbumMenu>
                      ))}
                    </div>
                  </section>
                ))}
              </div>
            )}
            {featuredAppearances.length > 0 && (
              <FeaturedAppearances appearances={featuredAppearances} />
            )}
            {!hasDiscography && (
              <p className="text-sm text-zinc-500">
                No albums by {name} are in this library.
              </p>
            )}
          </div>
        )}
        {artist &&
          (activeTab === "overview" || activeTab === "discography") && (
            <SimilarArtistsRow artistId={artist.id} name={artist.name} />
          )}
      </div>
    </div>
  );
}

/** "Similar artists" from shared genres in the library. */
function SimilarArtistsRow({
  artistId,
  name,
}: {
  artistId: string;
  name: string;
}) {
  const similar = useQuery({
    queryKey: ["artist-similar", artistId],
    queryFn: () => getSimilarArtists(artistId),
    staleTime: 5 * 60_000,
  });
  if (!similar.data?.length) return null;
  return (
    <section className="mt-16">
      <ShelfRow
        eyebrow={`If you like ${name}`}
        size="sm"
        title="Similar artists"
      >
        {similar.data.map((other) => (
          <ArtistCircle artist={other} key={other.id} />
        ))}
      </ShelfRow>
    </section>
  );
}

/** "2006–2012": the span of an artist's dated releases. */
function artistYears(albums: Album[]) {
  const years = albums
    .map((album) => Number(/^\d{4}/.exec(album.first_release_date || "")?.[0]))
    .filter((year) => Number.isFinite(year) && year > 0);
  if (!years.length) return "";
  const first = Math.min(...years);
  const last = Math.max(...years);
  return first === last ? String(first) : `${first}–${last}`;
}

/**
 * The artist's photo: a soft, blurred wash of it across the whole header,
 * and the photo itself large on the right, fading into the canvas behind
 * the name, so even a small Commons portrait reads sharp.
 */
function ArtistPhotoHero({ src }: { src: string }) {
  return (
    <div
      aria-hidden
      className="absolute inset-0 -z-10 overflow-hidden"
      data-artist-hero="photo"
    >
      <Image
        alt=""
        className="scale-110 object-cover opacity-35 blur-3xl saturate-[0.8]"
        fill
        sizes="40vw"
        src={src}
      />
      <div className="absolute inset-y-0 right-0 w-full md:w-[64%]">
        <Image
          alt=""
          className="object-cover object-[50%_22%] [mask-image:linear-gradient(to_right,transparent,black_38%)] max-md:opacity-60 max-md:[mask-image:none]"
          fill
          priority
          sizes="(min-width: 768px) 64vw, 100vw"
          src={src}
        />
      </div>
      <div className="absolute inset-0 bg-[linear-gradient(to_top,var(--color-mujin-base)_0%,rgba(10,10,11,0.55)_38%,rgba(10,10,11,0)_75%),linear-gradient(to_right,rgba(10,10,11,0.92)_0%,rgba(10,10,11,0.55)_45%,transparent_80%),radial-gradient(60%_80%_at_80%_20%,rgba(217,139,95,0.12),transparent_70%)]" />
    </div>
  );
}

/**
 * Without a photo, the hero is a mosaic of the artist's covers: large,
 * slightly tilted, softened into the canvas.
 */
function ArtistMosaic({ covers }: { covers: string[] }) {
  const tiles = Array.from(
    { length: 8 },
    (_, index) => covers[index % covers.length]!,
  );
  return (
    <div aria-hidden className="absolute inset-0 -z-10 overflow-hidden">
      <div className="absolute -inset-x-[10%] -top-[30%] bottom-0 grid rotate-[-6deg] grid-cols-4 gap-3 opacity-55 saturate-[0.85]">
        {tiles.map((cover, index) => (
          <div
            className="relative aspect-square overflow-hidden rounded-md"
            key={`${cover}-${index}`}
          >
            <Image
              alt=""
              className="object-cover"
              fill
              sizes="25vw"
              src={cover}
            />
          </div>
        ))}
      </div>
      <div className="absolute inset-0 bg-[linear-gradient(to_top,var(--color-mujin-base)_5%,rgba(10,10,11,0.82)_45%,rgba(10,10,11,0.45)_100%),linear-gradient(to_right,rgba(10,10,11,0.9),rgba(10,10,11,0.2)_75%)]" />
    </div>
  );
}

function FeaturedAppearances({
  appearances,
}: {
  appearances: { album: LibraryAlbum; track: LibraryAlbum["songs"][number] }[];
}) {
  return (
    <section>
      <h3 className="mb-5 text-xl font-semibold text-white">
        Tracks they appear on
      </h3>
      <div className="grid gap-3 sm:grid-cols-2">
        {appearances.map(({ album, track }) => {
          const coverURL = getLibraryImageUrl(album.cover_url, getBaseURL);
          return (
            <Link
              className="group flex min-w-0 items-center gap-4 rounded-lg p-2 transition-colors hover:bg-white/6"
              href={`/album?id=${album.id}`}
              key={`${album.id}-${track.id}`}
            >
              <div className="relative h-16 w-16 shrink-0 overflow-hidden rounded-md border border-white/10 bg-white/4">
                {coverURL ? (
                  <Image
                    alt={album.name}
                    className="object-cover transition-transform duration-300 group-hover:scale-105"
                    fill
                    sizes="64px"
                    src={coverURL}
                  />
                ) : (
                  <AlbumArtFallback />
                )}
              </div>
              <div className="min-w-0">
                <h4 className="truncate text-sm font-semibold text-zinc-100">
                  <SongTitle title={track.name} />
                </h4>
                <p className="mt-1 truncate text-sm text-zinc-400">
                  {isExplicitSongTitle(track.name) && <ExplicitBadge />}
                  {album.artist_object.name} · {album.name}
                </p>
              </div>
            </Link>
          );
        })}
      </div>
    </section>
  );
}
