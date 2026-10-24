import type {
  StreamingAlbum,
  StreamingArtistSummary,
  StreamingProviderId,
  StreamingProviderStatus,
  StreamingSearchResponse,
  StreamingTrack,
} from "@parson/music-sdk";

export type StreamingSearchRow =
  | {
      type: "service";
      key: string;
      provider: StreamingProviderId;
      name: string;
      error?: string;
      empty: boolean;
    }
  | { type: "album"; key: string; album: StreamingAlbum }
  | {
      type: "track";
      key: string;
      track: StreamingTrack;
      queue: StreamingTrack[];
    }
  | { type: "artist"; key: string; artist: StreamingArtistSummary };

/** Flattens per-service search results into list rows: header, artists, albums, tracks. */
export function streamingSearchRows(
  response: StreamingSearchResponse | undefined,
  limits = { artists: 3, albums: 6, tracks: 6 },
): StreamingSearchRow[] {
  const rows: StreamingSearchRow[] = [];
  for (const service of response?.services ?? []) {
    const results = service.results;
    const artists = results?.artists.slice(0, limits.artists) ?? [];
    const albums = results?.albums.slice(0, limits.albums) ?? [];
    const tracks = (results?.tracks.slice(0, limits.tracks) ?? []).filter(
      (track) => track.streamable,
    );
    rows.push({
      type: "service",
      key: `service-${service.provider}`,
      provider: service.provider,
      name: service.name,
      error: service.error,
      empty:
        !service.error && !artists.length && !albums.length && !tracks.length,
    });
    for (const artist of artists) {
      rows.push({ type: "artist", key: `artist-${artist.id}`, artist });
    }
    for (const album of albums) {
      rows.push({ type: "album", key: `album-${album.id}`, album });
    }
    for (const track of tracks) {
      rows.push({
        type: "track",
        key: `track-${track.id}`,
        track,
        queue: tracks,
      });
    }
  }
  return rows;
}

export function serviceAction(provider: StreamingProviderStatus) {
  if (provider.status === "signed_in") return "Manage";
  return provider.credentials.configured ? "Sign in" : "Set up";
}

/** Seconds until a TIDAL device code expires, never negative. */
export function codeSecondsLeft(expiresAt: number, nowMs = Date.now()) {
  return Math.max(0, expiresAt - Math.floor(nowMs / 1000));
}
