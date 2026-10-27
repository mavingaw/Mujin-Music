import {
  liveStationTrack,
  type Album,
  type Artist,
  type LibrarySong,
  type LiveStation,
} from "@parson/music-sdk";

/** A station logo as an absolute URL on the connected server. */
export function stationLogoUrl(
  station: Pick<LiveStation, "logo_url">,
  origin: string | null | undefined,
): string | null {
  const logo = station.logo_url;
  if (!logo) return null;
  if (/^https?:\/\//i.test(logo)) return logo;
  if (!origin) return null;
  return `${origin.replace(/\/+$/, "")}${logo.startsWith("/") ? "" : "/"}${logo}`;
}

/** A station as a song the native player can queue (duration 0 = live). */
export function stationSong(
  station: LiveStation,
  origin: string | null | undefined,
): LibrarySong {
  const track = liveStationTrack(station, stationLogoUrl(station, origin));
  const artist: Artist = {
    albums: [],
    description: "",
    featured_on_album_ids: [],
    followers: 0,
    icon_url: "",
    id: "",
    name: track.artist,
  };
  const album: Album = {
    contributing_artists: [],
    contributing_artists_ids: [],
    cover_url: track.album_object.cover_url,
    description: "",
    first_release_date: "",
    id: "",
    musicbrainz_id: "",
    name: track.album_object.name,
    primary_type: "",
    songs: [],
    wikidata_id: null,
  };
  return {
    ...track,
    album_id: "",
    artist_id: "",
    album_object: album,
    artist_object: artist,
  };
}

/** Reconnect attempts a dropped live stream gets per minute. */
export const LIVE_RECONNECTS_PER_MINUTE = 5;

/**
 * Whether a live stream that ended should reconnect: it only ends when the
 * connection drops, but a station that keeps failing is left alone.
 */
export function shouldReconnect(
  attempts: number[],
  now: number,
): { reconnect: boolean; attempts: number[] } {
  const recent = attempts.filter((at) => now - at < 60_000);
  if (recent.length >= LIVE_RECONNECTS_PER_MINUTE) {
    return { reconnect: false, attempts: recent };
  }
  return { reconnect: true, attempts: [...recent, now] };
}
