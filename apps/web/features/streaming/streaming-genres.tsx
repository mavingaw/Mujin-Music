"use client";

import {
  getStreamingGenreAlbums,
  getStreamingGenres,
  type StreamingBrowseSectionInfo,
  type StreamingEditorialSection,
  type StreamingGenre,
  type StreamingProviderId,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { ChevronLeft, Loader2 } from "lucide-react";
import { useState } from "react";
import { cn } from "@/lib/cn";
import AlbumCard from "@/features/library/album-card";
import { genreTileStyle } from "./streaming-model";

function Loading() {
  return (
    <p className="flex items-center gap-2 text-sm text-zinc-500">
      <Loader2 className="h-4 w-4 animate-spin" /> Loading…
    </p>
  );
}

function Failed({ onRetry }: { onRetry: () => void }) {
  return (
    <p className="text-sm text-zinc-400">
      This section could not be loaded.{" "}
      <button
        className="text-white underline-offset-4 hover:underline"
        onClick={onRetry}
        type="button"
      >
        Try again
      </button>
    </p>
  );
}

function GenreTile({
  genre,
  onOpen,
}: {
  genre: StreamingGenre;
  onOpen: () => void;
}) {
  return (
    <button
      className="group relative flex aspect-[16/9] items-end overflow-hidden rounded-xl p-4 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white"
      onClick={onOpen}
      style={genreTileStyle(genre.name, genre.image_url)}
      type="button"
    >
      <span className="absolute inset-0 bg-linear-to-t from-black/70 via-black/10 to-transparent transition-opacity group-hover:opacity-80" />
      <span className="relative text-lg font-semibold leading-tight text-white drop-shadow">
        {genre.name}
      </span>
    </button>
  );
}

function GenreAlbums({
  provider,
  genre,
  sections,
}: {
  provider: StreamingProviderId;
  genre: StreamingGenre;
  sections: StreamingBrowseSectionInfo[];
}) {
  const byGenre = sections.filter((section) => section.by_genre);
  const [section, setSection] = useState<StreamingEditorialSection>(
    byGenre[0]?.id ?? "new_releases",
  );
  const page = useQuery({
    queryKey: ["streaming", "genre", provider, genre.id, section],
    queryFn: () =>
      getStreamingGenreAlbums(provider, genre.id, { section, limit: 50 }),
  });
  return (
    <div className="space-y-5">
      {byGenre.length > 1 && (
        <div
          aria-label={`${genre.name} lists`}
          className="flex flex-wrap gap-2"
          role="group"
        >
          {byGenre.map((item) => (
            <button
              aria-pressed={section === item.id}
              className={cn(
                "inline-flex h-8 items-center rounded-full border px-3 text-[0.8125rem] font-medium",
                section === item.id
                  ? "border-white bg-white text-black"
                  : "border-white/12 text-zinc-300 hover:text-white",
              )}
              key={item.id}
              onClick={() => setSection(item.id)}
              type="button"
            >
              {item.label}
            </button>
          ))}
        </div>
      )}
      {provider === "tidal" && (
        <p className="max-w-2xl text-[0.8125rem] leading-5 text-zinc-500">
          TIDAL lists albums by genre only on its genre pages, so Mujin Music
          files an album you add from here under {genre.name} in your library.
        </p>
      )}
      {page.isPending ? (
        <Loading />
      ) : page.isError || !page.data ? (
        <Failed onRetry={() => void page.refetch()} />
      ) : page.data.albums.length ? (
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
      ) : (
        <p className="text-sm text-zinc-500">
          No {genre.name} albums in this list right now.
        </p>
      )}
    </div>
  );
}

/** A service's genres as tiles, then one genre's albums. */
export function StreamingGenresSection({
  provider,
  sections,
  genreId,
  onGenreChange,
}: {
  provider: StreamingProviderId;
  sections: StreamingBrowseSectionInfo[];
  genreId: string | null;
  onGenreChange: (genreId: string | null) => void;
}) {
  const genres = useQuery({
    queryKey: ["streaming", "genres", provider],
    queryFn: () => getStreamingGenres(provider),
    staleTime: 60 * 60 * 1000,
  });
  if (genres.isPending) return <Loading />;
  if (genres.isError || !genres.data) {
    return <Failed onRetry={() => void genres.refetch()} />;
  }
  const genre = genres.data.find((item) => item.id === genreId);
  if (genre) {
    return (
      <div className="space-y-5">
        <div className="flex items-center gap-3">
          <button
            aria-label="All genres"
            className="grid h-9 w-9 place-items-center rounded-full border border-white/12 text-zinc-300 hover:text-white"
            onClick={() => onGenreChange(null)}
            type="button"
          >
            <ChevronLeft className="h-4 w-4" />
          </button>
          <h2 className="text-2xl font-semibold text-white">{genre.name}</h2>
        </div>
        <GenreAlbums genre={genre} provider={provider} sections={sections} />
      </div>
    );
  }
  if (!genres.data.length) {
    return <p className="text-sm text-zinc-500">No genres to browse.</p>;
  }
  return (
    <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
      {genres.data.map((item) => (
        <GenreTile
          genre={item}
          key={item.id}
          onOpen={() => onGenreChange(item.id)}
        />
      ))}
    </div>
  );
}
