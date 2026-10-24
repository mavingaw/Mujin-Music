"use client";

import {
  STREAMING_PROVIDER_NAMES,
  getStreamingArtist,
  parseStreamingId,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { ExternalLink, Play, UserRound } from "lucide-react";
import Image from "next/image";
import { usePageTitle } from "@/components/app/title-metadata";
import AlbumCard from "@/features/library/album-card";
import EntityPageState from "@/features/library/entity-page-state";
import { SourceBadge } from "./source-badge";
import { StreamingTrackRow } from "./streaming-track-row";
import { useStreamingPlayback } from "./use-streaming";

export default function StreamingArtistDetails({ id }: { id: string }) {
  const parsed = parseStreamingId(id);
  const artist = useQuery({
    queryKey: ["streaming", "artist", id],
    queryFn: () => getStreamingArtist(id),
    enabled: Boolean(parsed),
    retry: 1,
  });
  const playback = useStreamingPlayback();
  const data = artist.data;
  usePageTitle(data?.name ?? null);

  if (!parsed) return <EntityPageState kind="artist" />;
  if (artist.isPending) return <EntityPageState kind="artist" loading />;
  if (artist.isError || !data) {
    return (
      <EntityPageState kind="artist" onRetry={() => void artist.refetch()} />
    );
  }

  return (
    <div className="mx-auto max-w-266 px-5 pb-24 pt-8 sm:px-7">
      <header className="mb-10 flex flex-col items-start gap-6 sm:flex-row sm:items-end">
        <div className="relative flex h-40 w-40 shrink-0 items-center justify-center overflow-hidden rounded-full bg-white/5 sm:h-48 sm:w-48">
          {data.picture_url ? (
            <Image
              alt={data.name}
              className="object-cover"
              fill
              sizes="192px"
              src={data.picture_url}
            />
          ) : (
            <UserRound className="h-14 w-14 text-zinc-600" />
          )}
        </div>
        <div className="min-w-0">
          <SourceBadge provider={data.provider} />
          <h1 className="mt-3 text-4xl font-black text-white sm:text-6xl">
            {data.name}
          </h1>
          <div className="mt-5 flex items-center gap-4">
            {data.top_tracks.length > 0 && (
              <button
                className="inline-flex h-11 items-center gap-2 rounded-full bg-white px-6 text-sm font-semibold text-black hover:bg-zinc-200"
                onClick={() => playback.playTracks(data.top_tracks)}
                type="button"
              >
                <Play className="h-4 w-4 fill-black" /> Play top tracks
              </button>
            )}
            {data.web_url && (
              <a
                className="inline-flex items-center gap-1.5 text-sm text-zinc-500 hover:text-zinc-200"
                href={data.web_url}
                rel="noreferrer"
                target="_blank"
              >
                Open in {STREAMING_PROVIDER_NAMES[data.provider]}
                <ExternalLink className="h-3.5 w-3.5" />
              </a>
            )}
          </div>
        </div>
      </header>

      {data.top_tracks.length > 0 && (
        <section className="mb-12">
          <h2 className="mb-3 text-xl font-semibold text-white">Top tracks</h2>
          <div className="space-y-1">
            {data.top_tracks.map((track, index) => (
              <StreamingTrackRow
                active={playback.currentSongId === track.id}
                index={index}
                key={track.id}
                onPlay={() => playback.playTracks(data.top_tracks, index)}
                playing={
                  playback.isPlaying && playback.currentSongId === track.id
                }
                showAlbum
                track={{ ...track, track_number: index + 1 }}
              />
            ))}
          </div>
        </section>
      )}

      <section>
        <h2 className="mb-5 text-xl font-semibold text-white">Albums</h2>
        {data.albums.length ? (
          <div className="grid grid-cols-2 gap-x-4 gap-y-10 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
            {data.albums.map((album) => (
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
          <p className="text-sm text-zinc-500">No albums found.</p>
        )}
      </section>
    </div>
  );
}
