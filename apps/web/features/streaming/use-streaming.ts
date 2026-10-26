"use client";

import {
  addToStreamingLibrary,
  getStreamingProviders,
  removeFromStreamingLibrary,
  streamingAlbumToAlbum,
  streamingTrackToSong,
  type StreamingAlbum,
  type StreamingItemKind,
  type StreamingTrack,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { usePlayer } from "@/features/player/player-context";
import { connectedProviders } from "./streaming-model";

export const streamingProvidersKey = ["streaming", "providers"] as const;

export function useStreamingProviders(enabled = true) {
  const query = useQuery({
    queryKey: streamingProvidersKey,
    queryFn: getStreamingProviders,
    enabled,
    staleTime: 30_000,
  });
  return {
    ...query,
    providers: query.data?.providers ?? [],
    connected: connectedProviders(query.data?.providers),
    sandbox: query.data?.sandbox ?? false,
  };
}

const errorMessage = (error: unknown, fallback: string) => {
  const data = (error as { response?: { data?: { message?: string } } })
    ?.response?.data;
  return data?.message ?? fallback;
};

export { errorMessage as streamingErrorMessage };

/** Play, queue and library actions for streaming albums and tracks. */
export function useStreamingPlayback() {
  const {
    addToQueue,
    isPlaying,
    playAudioSource,
    setCurrentSongIndex,
    setQueue,
    setSongCallback,
    song: currentSong,
    togglePlayPause,
  } = usePlayer();

  const playAlbum = (album: StreamingAlbum, trackId?: string) => {
    const converted = streamingAlbumToAlbum(album);
    const playable = converted.songs.filter(
      (_, index) => album.tracks[index]?.streamable !== false,
    );
    if (!playable.length) {
      toast("This album has no playable tracks.");
      return;
    }
    const index = Math.max(
      0,
      playable.findIndex((song) => song.id === trackId),
    );
    if (trackId && currentSong?.id === trackId) {
      togglePlayPause();
      return;
    }
    const artist = converted.artist_object;
    const first = playable[index];
    if (!first) return;
    setQueue(playable.map((song) => ({ song, artist, album: converted })));
    setCurrentSongIndex(index);
    setSongCallback(first, artist, converted);
    playAudioSource();
  };

  const playTracks = (tracks: StreamingTrack[], index = 0) => {
    const songs = tracks
      .filter((track) => track.streamable)
      .map((track) => streamingTrackToSong(track));
    if (!songs.length) {
      toast("Nothing here can be played.");
      return;
    }
    const start = Math.min(index, songs.length - 1);
    const first = songs[start];
    if (!first) return;
    if (currentSong?.id === first.id) {
      togglePlayPause();
      return;
    }
    setQueue(
      songs.map((song) => ({
        song,
        artist: song.artist_object,
        album: song.album_object,
      })),
    );
    setCurrentSongIndex(start);
    setSongCallback(first, first.artist_object, first.album_object);
    playAudioSource();
  };

  const queueAlbum = (album: StreamingAlbum) => {
    const converted = streamingAlbumToAlbum(album);
    addToQueue(
      converted.songs.map((song) => ({
        song,
        artist: converted.artist_object,
        album: converted,
      })),
    );
    toast.success(`Added ${album.title} to queue`);
  };

  return {
    currentSongId: currentSong?.id,
    currentAlbumId: currentSong?.album_object?.id,
    isPlaying,
    playAlbum,
    playTracks,
    queueAlbum,
  };
}

/** Toggles a streaming album or track in the listener's library. */
export function useStreamingLibraryToggle() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({
      kind,
      id,
      add,
    }: {
      kind: StreamingItemKind;
      id: string;
      add: boolean;
      title: string;
    }) =>
      add
        ? addToStreamingLibrary(kind, id)
        : removeFromStreamingLibrary(kind, id),
    onSuccess: async (change, { add, title }) => {
      if (add) {
        toast.success(
          change.service_updated
            ? `Added ${title} to your library`
            : `Added ${title} to your library. The service favorite could not be updated.`,
        );
      } else {
        toast(`Removed ${title} from your library`);
      }
      await queryClient.invalidateQueries({ queryKey: ["streaming"] });
    },
    onError: (error) =>
      toast(errorMessage(error, "Could not update your library.")),
  });
}
