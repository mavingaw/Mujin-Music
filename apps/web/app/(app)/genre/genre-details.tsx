"use client";

import getBaseURL from "@/lib/api/server-url";
import { AlbumArtFallback, getLibraryImageUrl } from "@/lib/images/image-url";
import {
  flattenGenreTree,
  genreAboutWaitingLabel,
  genreHref,
} from "@parson/music-sdk";
import type {
  GenreAlbum,
  GenreCount,
  GenrePerson,
  GenreTreeNode,
} from "@parson/music-sdk/types";
import { ChevronRight, ExternalLink, Loader2, Tags } from "lucide-react";
import Image from "next/image";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import type { ReactNode } from "react";
import { usePageTitle } from "@/components/app/title-metadata";
import { EditorialText } from "@/features/library/credits/editorial-text";
import { PersonAvatar } from "@/features/library/credits/person-avatar";
import { GenreArt } from "@/features/library/artist-imagery";
import {
  useGenrePage,
  useGenreTree,
} from "@/features/library/genres/use-genres";

const SOURCE_NOTE = {
  online:
    "Genres from MusicBrainz, or from file tags where MusicBrainz has none.",
  tags: "Genres from file tags, or from MusicBrainz where the files have none.",
};

function albumsLabel(count: number) {
  return `${count.toLocaleString()} ${count === 1 ? "album" : "albums"}`;
}

function GenrePill({ genre }: { genre: GenreCount }) {
  const empty = genre.album_count === 0;
  return (
    <Link
      className={`inline-flex items-center gap-2 rounded-full border px-3 py-1 text-sm transition-colors ${
        empty
          ? "border-white/6 text-zinc-500 hover:text-zinc-300"
          : "border-white/12 bg-white/5 text-zinc-100 hover:border-white/25 hover:bg-white/10"
      }`}
      href={genreHref(genre.slug)}
    >
      {genre.name}
      {!empty && (
        <span className="text-xs tabular-nums text-zinc-500">
          {genre.album_count}
        </span>
      )}
    </Link>
  );
}

function AlbumCard({ album }: { album: GenreAlbum }) {
  const cover = getLibraryImageUrl(album.cover_url, getBaseURL);
  return (
    <li className="min-w-0">
      <Link className="group block" href={`/album?id=${album.id}`}>
        <div className="relative aspect-square overflow-hidden rounded-md border border-white/8 bg-white/4">
          {cover ? (
            <Image
              alt={album.name}
              className="object-cover transition-transform duration-300 group-hover:scale-[1.03]"
              fill
              sizes="(min-width: 768px) 180px, 45vw"
              src={cover}
            />
          ) : (
            <AlbumArtFallback />
          )}
        </div>
        <p className="mt-2 truncate text-sm font-semibold text-white group-hover:underline">
          {album.name}
        </p>
      </Link>
      <p className="truncate text-xs text-zinc-400">
        <Link
          className="hover:text-white hover:underline"
          href={`/artist?id=${encodeURIComponent(album.artist_id)}`}
        >
          {album.artist_name}
        </Link>
        {album.year && ` · ${album.year}`}
      </p>
    </li>
  );
}

function PersonCard({
  person,
  detail,
}: {
  person: GenrePerson;
  detail: string;
}) {
  return (
    <li className="min-w-0">
      <Link
        className="group flex flex-col items-center text-center"
        href={`/artist?id=${encodeURIComponent(person.artist_id)}`}
      >
        <PersonAvatar
          className="transition-transform duration-300 group-hover:scale-[1.03]"
          // The library's own picture is one of their covers: initials
          // read more honestly than a cover posing as a portrait.
          imageUrl={person.image_url}
          name={person.name}
          size={112}
        />
        <span className="mt-2 line-clamp-2 text-sm font-semibold text-white group-hover:underline">
          {person.name}
        </span>
        <span className="text-xs text-zinc-500">{detail}</span>
      </Link>
    </li>
  );
}

function Section({
  id,
  title,
  count,
  children,
}: {
  id: string;
  title: string;
  count?: number;
  children: ReactNode;
}) {
  return (
    <section aria-labelledby={id} className="mb-12">
      <h2 className="mb-4 text-2xl font-bold text-white" id={id}>
        {title}
        {count != null && (
          <span className="ml-2 text-base font-medium text-zinc-500">
            {count.toLocaleString()}
          </span>
        )}
      </h2>
      {children}
    </section>
  );
}

function TreeBranch({ node }: { node: GenreTreeNode }) {
  return (
    <li className="overflow-hidden rounded-xl border border-white/8 bg-white/2">
      <Link
        className="group relative block aspect-[16/7] overflow-hidden"
        href={genreHref(node.slug)}
      >
        <GenreArt
          covers={node.covers}
          name={node.name}
          photos={node.photos}
          sizes="(min-width: 640px) 450px, 100vw"
        />
        <span
          aria-hidden="true"
          className="absolute inset-0 z-[2] bg-[linear-gradient(to_top,rgba(10,10,11,0.92)_0%,rgba(10,10,11,0.35)_60%,rgba(10,10,11,0.05)_100%)]"
        />
        <span className="absolute inset-x-4 bottom-3 z-[3] flex items-baseline justify-between gap-3">
          <span className="mj-title truncate text-[24px] leading-tight text-mujin-text group-hover:underline">
            {node.name}
          </span>
          <span className="mj-tnum shrink-0 text-xs font-medium text-mujin-text-secondary">
            {albumsLabel(node.album_count)}
          </span>
        </span>
      </Link>
      {node.children.length > 0 && (
        <ul className="flex flex-wrap gap-1.5 p-4">
          {flattenGenreTree(node.children).map((child) => (
            <li key={child.slug}>
              <GenrePill genre={child} />
            </li>
          ))}
        </ul>
      )}
    </li>
  );
}

/** Every genre in the library, as Roon's genre browser: a tree of chips. */
function GenreIndex() {
  const tree = useGenreTree();
  usePageTitle("Genres");
  return (
    <div className="tidal-route relative text-zinc-50">
      <div className="relative mx-auto max-w-225 px-5 pb-24 pt-10 sm:px-7">
        <header className="mb-8">
          <p className="mb-3 flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.14em] text-zinc-500">
            <Tags className="h-3.5 w-3.5" /> Library
          </p>
          <h1 className="text-4xl mj-display text-mujin-text sm:text-5xl">
            Genres
          </h1>
          {tree.data && (
            <p className="mt-3 text-sm text-zinc-400">
              {SOURCE_NOTE[tree.data.source]}
            </p>
          )}
        </header>
        {tree.isPending && (
          <div className="flex justify-center pt-16 text-zinc-500">
            <Loader2 className="h-6 w-6 animate-spin" />
          </div>
        )}
        {tree.data &&
          tree.data.genres.length === 0 &&
          tree.data.other.length === 0 && (
            <p className="text-sm text-zinc-500">
              No album in your library has a genre yet. Genres come from the
              files&apos; GENRE tags and, with online metadata on, from
              MusicBrainz.
            </p>
          )}
        <ul className="grid gap-3 sm:grid-cols-2">
          {tree.data?.genres.map((node) => (
            <TreeBranch key={node.slug} node={node} />
          ))}
        </ul>
        {tree.data && tree.data.other.length > 0 && (
          <section className="mt-10">
            <h2 className="mb-3 text-sm font-semibold uppercase tracking-[0.08em] text-zinc-400">
              More genres
            </h2>
            <ul className="flex flex-wrap gap-1.5">
              {tree.data.other.map((genre) => (
                <li key={genre.slug}>
                  <GenrePill genre={genre} />
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>
    </div>
  );
}

/**
 * A genre: its place in the genre tree, an About text from Wikipedia (found
 * through Wikidata, with attribution), sub-genres, and the library's
 * artists, composers and albums in it or any of its sub-genres.
 */
export default function GenreDetails() {
  const searchParams = useSearchParams();
  const slug = searchParams?.get("slug");
  const query = useGenrePage(slug);
  const page = query.data;
  usePageTitle(page ? page.name : null);

  if (!slug) return <GenreIndex />;
  if (query.isPending) {
    return (
      <div className="flex justify-center pt-24 text-zinc-500">
        <Loader2 className="h-6 w-6 animate-spin" />
      </div>
    );
  }
  if (query.isError || !page) {
    return (
      <div className="mx-auto max-w-225 px-7 pt-16 text-zinc-400">
        <p>No album in your library has this genre.</p>
        <Link
          className="mt-3 inline-block text-sm text-white underline-offset-2 hover:underline"
          href="/genre"
        >
          All genres
        </Link>
      </div>
    );
  }

  const waiting = genreAboutWaitingLabel(page.enrichment);
  const subGenres = page.children.filter(
    (child) => child.album_count > 0 || page.children.length <= 12,
  );
  const heroPhotos = [...page.artists, ...page.composers]
    .map((person) => person.image_url)
    .filter((url): url is string => Boolean(url))
    .filter((url, index, all) => all.indexOf(url) === index)
    .slice(0, 3);
  const hero = page.albums
    .map((album) => getLibraryImageUrl(album.cover_url, getBaseURL))
    .filter((url): url is string => Boolean(url))
    .slice(0, 5);

  return (
    <div className="tidal-route relative text-zinc-50">
      <header className="relative isolate mb-10 overflow-hidden rounded-2xl border border-white/8 bg-zinc-900/60">
        {(hero.length > 0 || heroPhotos.length > 0) && (
          <div aria-hidden className="absolute inset-0 -z-10">
            {/* Its artists' photos (or its covers), duotone in copper. */}
            <GenreArt
              covers={page.albums.map((album) => album.cover_url)}
              name={page.name}
              photos={heroPhotos}
              sizes="(min-width: 768px) 300px, 100vw"
            />
            <div className="absolute inset-0 z-[2] bg-linear-to-t from-black/90 via-black/55 to-black/20" />
          </div>
        )}
        <div className="px-6 pb-7 pt-12 sm:px-8 sm:pt-16">
          <nav
            aria-label="Genre path"
            className="mb-3 flex flex-wrap items-center gap-1 text-xs font-semibold uppercase tracking-[0.14em] text-zinc-400"
          >
            <Link className="hover:text-white" href="/genre">
              Genres
            </Link>
            {page.path.map((ancestor) => (
              <span className="flex items-center gap-1" key={ancestor.slug}>
                <ChevronRight className="h-3 w-3" />
                <Link
                  className="hover:text-white"
                  href={genreHref(ancestor.slug)}
                >
                  {ancestor.name}
                </Link>
              </span>
            ))}
          </nav>
          <h1 className="text-balance text-4xl mj-display leading-none text-mujin-text sm:text-6xl">
            {page.name}
          </h1>
          <p className="mt-3 text-sm text-zinc-300">
            {[
              albumsLabel(page.album_count),
              page.artists.length &&
                `${page.artists.length} ${page.artists.length === 1 ? "artist" : "artists"}`,
              page.composers.length &&
                `${page.composers.length} ${page.composers.length === 1 ? "composer" : "composers"}`,
            ]
              .filter(Boolean)
              .join(" · ")}
          </p>
          {subGenres.length > 0 && (
            <ul aria-label="Sub-genres" className="mt-5 flex flex-wrap gap-1.5">
              {subGenres.map((child) => (
                <li key={child.slug}>
                  <GenrePill genre={child} />
                </li>
              ))}
            </ul>
          )}
        </div>
      </header>

      <div className="relative mx-auto max-w-225 px-1 pb-24">
        <section aria-labelledby="genre-about" className="mb-12">
          <h2
            className="mb-3 text-sm font-semibold uppercase tracking-[0.08em] text-zinc-400"
            id="genre-about"
          >
            About
          </h2>
          {page.about ? (
            <EditorialText clampLines={5} text={page.about} />
          ) : (
            <p className="text-sm text-zinc-500">
              {waiting ??
                (!page.canonical
                  ? "This genre comes from your files' tags; Mujin Music has no description for it."
                  : page.enrichment.enabled
                    ? "No Wikipedia description was found for this genre."
                    : "Turn on online metadata in Settings to add a description from Wikipedia.")}
            </p>
          )}
          {page.links.length > 0 && (
            <p className="mt-3 flex gap-4 text-xs">
              {page.links.map((link) => (
                <a
                  className="inline-flex items-center gap-1 text-zinc-500 hover:text-white"
                  href={link.url}
                  key={link.url}
                  rel="noreferrer"
                  target="_blank"
                >
                  {link.provider === "wikidata" ? "Wikidata" : link.provider}
                  <ExternalLink className="h-3 w-3" />
                </a>
              ))}
            </p>
          )}
        </section>

        {page.artists.length > 0 && (
          <Section
            count={page.artists.length}
            id="genre-artists"
            title="Artists"
          >
            <ul className="grid grid-cols-2 gap-x-4 gap-y-6 sm:grid-cols-4 md:grid-cols-5">
              {page.artists.slice(0, 15).map((person) => (
                <PersonCard
                  detail={albumsLabel(person.album_count)}
                  key={person.artist_id}
                  person={person}
                />
              ))}
            </ul>
          </Section>
        )}

        {page.composers.length > 0 && (
          <Section
            count={page.composers.length}
            id="genre-composers"
            title="Composers"
          >
            <ul className="grid grid-cols-2 gap-x-4 gap-y-6 sm:grid-cols-4 md:grid-cols-5">
              {page.composers.slice(0, 10).map((person) => (
                <PersonCard
                  detail={`${person.track_count.toLocaleString()} ${person.track_count === 1 ? "track" : "tracks"}`}
                  key={person.artist_id}
                  person={person}
                />
              ))}
            </ul>
          </Section>
        )}

        <Section count={page.album_count} id="genre-albums" title="Your albums">
          {page.albums.length === 0 ? (
            <p className="text-sm text-zinc-500">
              No album in your library is in this genre yet.
            </p>
          ) : (
            <ul className="grid grid-cols-2 gap-x-4 gap-y-6 sm:grid-cols-4 md:grid-cols-5">
              {page.albums.map((album) => (
                <AlbumCard album={album} key={album.id} />
              ))}
            </ul>
          )}
        </Section>

        {page.related.length > 0 && (
          <section aria-labelledby="genre-related" className="mb-12">
            <h2
              className="mb-3 text-sm font-semibold uppercase tracking-[0.08em] text-zinc-400"
              id="genre-related"
            >
              Related genres
            </h2>
            <ul className="flex flex-wrap gap-1.5">
              {page.related.map((genre) => (
                <li key={genre.slug}>
                  <GenrePill genre={genre} />
                </li>
              ))}
            </ul>
          </section>
        )}
        <p className="text-xs text-zinc-600">{SOURCE_NOTE[page.source]}</p>
      </div>
    </div>
  );
}
