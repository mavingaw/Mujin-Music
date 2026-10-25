"use client";

import { AlbumArtFallback, getLibraryImageUrl } from "@/lib/images/image-url";
import getBaseURL from "@/lib/api/server-url";
import { Album, Artist } from "@parson/music-sdk/types";
import {
  getAlbumInfos,
  getApiBaseURL,
  isApiError,
  type LibraryAlbum,
} from "@parson/music-sdk";
import { useArtist } from "@/features/library/use-library-entity";
import { SongTitle, ExplicitBadge } from "@/features/library/song-title";
import { isExplicitSongTitle } from "@/features/library/song-presentation";
import Image from "next/image";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useMemo } from "react";
import { useSession } from "@/features/account/session-provider";
import {
  useArtistAbout,
  useArtistCredits,
} from "@/features/library/credits/use-editorial";
import { ArtistCreditsPanel } from "./artist-credits";
import { ArtistCompositionsPanel } from "./artist-compositions";
import { useArtistWorks } from "@/features/library/works/use-works";
import { ArtistOverview } from "./artist-overview";
import { PersonAvatar } from "@/features/library/credits/person-avatar";
import { lifeSpanFacts } from "@/features/library/credits/credit-roles";
import { GenreChips } from "@/features/library/genres/genre-chips";
import { useQuery } from "@tanstack/react-query";
import { usePageTitle } from "@/components/app/title-metadata";
import EntityPageState from "@/features/library/entity-page-state";
import AlbumMenu from "@/features/library/album-menu";
import { songCreditsArtist } from "@/features/library/artist-appearance";

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

export default function ArtistDetails({
  devArtist,
  devAlbums,
}: ArtistDetailsProps = {}) {
  const searchParams = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const { session } = useSession();
  const id = searchParams?.get("id");
  const requestedTab = searchParams?.get("tab") as ArtistTab | null;
  const roleFilter = searchParams?.get("role") ?? null;

  const artistQuery = useArtist(id, devArtist);
  const artistMissing =
    artistQuery.isError &&
    isApiError(artistQuery.error) &&
    artistQuery.error.response?.status === 404;
  const artist = artistQuery.data ?? null;
  const aboutQuery = useArtistAbout(devArtist ? null : id);
  const about = aboutQuery.data;
  const hasRoles = Boolean(about?.roles.length);
  const creditsQuery = useArtistCredits(
    devArtist ? null : id,
    hasRoles || artistMissing,
  );
  const isComposer = Boolean(
    about?.roles.some((role) => role.role === "composer"),
  );
  const worksQuery = useArtistWorks(devArtist ? null : id, isComposer);
  const hasCompositions = Boolean(worksQuery.data?.works.length);
  const workFilter = {
    form: searchParams?.get("form"),
    period: searchParams?.get("period"),
    key: searchParams?.get("key"),
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
    discographySections.length > 0 || featuredAppearances.length > 0;
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
  const defaultTab: ArtistTab = about?.biography
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
    for (const facet of ["form", "period", "key"]) params.delete(facet);
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
  const heroImageURL = artistIconURL ?? editorialImageURL;
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
  const kicker = isComposer
    ? "Composer"
    : !artist
      ? "Contributor"
      : about?.facts?.kind === "Group" || about?.facts?.kind === "Orchestra"
        ? about.facts.kind
        : "Artist";

  return (
    <div className="tidal-route relative text-zinc-50">
      <header
        aria-label={`${name} header`}
        className="relative isolate mb-8 overflow-hidden rounded-2xl border border-white/8 bg-zinc-900/60"
      >
        {heroImageURL ? (
          <div aria-hidden className="absolute inset-0 -z-10">
            <Image
              alt=""
              className="scale-125 object-cover object-[50%_25%] opacity-50 blur-2xl"
              fill
              sizes="920px"
              src={heroImageURL}
            />
            <div className="absolute inset-0 bg-linear-to-t from-black/85 via-black/45 to-black/10" />
          </div>
        ) : (
          <div
            aria-hidden
            className="absolute inset-0 -z-10 bg-linear-to-br from-zinc-800/70 via-zinc-900/60 to-black"
          />
        )}
        <div className="flex flex-col gap-6 px-6 pb-7 pt-10 sm:flex-row sm:items-end sm:px-8 sm:pt-16">
          <figure className="shrink-0">
            {heroImageURL ? (
              <div className="relative h-36 w-36 overflow-hidden rounded-full border border-white/15 shadow-2xl shadow-black/60 sm:h-48 sm:w-48">
                <Image
                  src={heroImageURL}
                  alt={name}
                  fill
                  className="object-cover object-[50%_20%]"
                  sizes="192px"
                  priority
                />
              </div>
            ) : (
              <PersonAvatar
                className="shadow-2xl shadow-black/60"
                name={name}
                size={160}
              />
            )}
          </figure>
          <div className="min-w-0 pb-1">
            <p className="mb-2 text-xs font-semibold uppercase tracking-[0.14em] text-zinc-400">
              {kicker}
            </p>
            <h1 className="min-w-0 wrap-break-word text-4xl font-black leading-none text-white sm:text-6xl">
              {name}
            </h1>
            {(subtitle || lifeSpan) && (
              <p className="mt-3 text-sm text-zinc-300">
                {[subtitle, lifeSpan].filter(Boolean).join(" · ")}
              </p>
            )}
            <GenreChips className="mt-3" genres={about?.genres} />
            {!artistIconURL && about?.image && (
              <p className="mt-2 text-xs text-zinc-400">
                Photo:{" "}
                <a
                  className="hover:text-zinc-200 hover:underline"
                  href={about.image.page_url}
                  rel="noreferrer"
                  target="_blank"
                >
                  {about.image.author}
                </a>
                ,{" "}
                {about.image.license_url ? (
                  <a
                    className="hover:text-zinc-200 hover:underline"
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

      <div className="relative z-10">
        {tabs.length > 1 && (
          <div
            aria-label="Artist sections"
            className="mb-8 flex gap-6 border-b border-white/10"
            role="tablist"
          >
            {tabs.map((tab) => (
              <button
                aria-selected={activeTab === tab.key}
                className={`-mb-px border-b-2 pb-3 text-sm font-semibold uppercase tracking-[0.08em] transition-colors ${
                  activeTab === tab.key
                    ? "border-white text-white"
                    : "border-transparent text-zinc-500 hover:text-zinc-200"
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
            onShowCredits={(role) => navigate("credits", role)}
          />
        )}

        {activeTab === "compositions" && worksQuery.data && (
          <ArtistCompositionsPanel
            filter={workFilter}
            onFilterChange={(filter) => {
              const params = new URLSearchParams(
                searchParams?.toString() ?? "",
              );
              params.set("tab", "compositions");
              for (const facet of ["form", "period", "key"] as const) {
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

        {activeTab === "discography" && artist && (
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
              <section>
                <h3 className="mb-5 text-xl font-semibold text-white">
                  Appears on
                </h3>
                <div className="grid gap-3 sm:grid-cols-2">
                  {featuredAppearances.map(({ album, track }) => {
                    const coverURL = getLibraryImageUrl(
                      album.cover_url,
                      getBaseURL,
                    );
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
                            {isExplicitSongTitle(track.name) && (
                              <ExplicitBadge />
                            )}
                            {album.artist_object.name} · {album.name}
                          </p>
                        </div>
                      </Link>
                    );
                  })}
                </div>
              </section>
            )}
            {!hasDiscography && (
              <p className="text-sm text-zinc-500">
                No albums by {name} are in this library.
              </p>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
