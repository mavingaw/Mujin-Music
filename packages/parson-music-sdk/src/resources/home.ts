import api from "../core/http";
import type { ResponseSong } from "../domain/types";
import type { ResponseAlbum } from "../domain/types";

/** The best format an album is available in, for its quality badge. */
export interface HomeAlbumQuality {
  codec: string | null;
  sample_rate: number | null;
  bit_depth: number | null;
  /** Every track with a known codec is lossless. */
  lossless: boolean;
  /** Every track is lossless and above CD resolution. */
  hi_res: boolean;
  /** More than one codec, or lossless mixed with lossy. */
  mixed?: boolean;
  /**
   * The badge text the server chose ("24/96 FLAC", "MP3 320", "Mixed");
   * the Library grid shows the same text for the same album.
   */
  label?: string;
}

/** A compact album for discovery rows (no track list). */
export interface HomeAlbum {
  id: string;
  name: string;
  cover_url: string;
  first_release_date: string;
  primary_type: string;
  artist: { id: string; name: string };
  track_count: number;
  duration_seconds: number;
  quality: HomeAlbumQuality | null;
  genres: string[];
}

export interface HomeDailyPick {
  album: HomeAlbum;
  /** "Because you played …" or "From your library". */
  reason: string;
}

export interface HomeArtist {
  id: string;
  name: string;
  /** One of the artist's covers (the fallback picture). */
  image_url: string;
  /**
   * API path of the artist's photo (`/artists/{id}/image`) when online
   * metadata found one; older servers leave this out.
   */
  photo_url?: string | null;
  album_count: number;
  reason: string | null;
}

/** An artist's densest run of releases ("In their prime"). */
export interface HomeBestYears {
  artist: { id: string; name: string };
  image_url: string;
  /** API path of the artist's photo, when there is one. */
  photo_url?: string | null;
  from_year: number;
  to_year: number;
  albums: HomeAlbum[];
}

export interface HomeGenre {
  name: string;
  /** Genre page id: `/genre?slug=…`. */
  slug: string;
  album_count: number;
  artist_count: number;
  covers: string[];
  /** API paths of up to three of the genre's artists' photos. */
  photos?: string[];
}

/** What the listener has actually played, all time. */
export interface HomeListening {
  plays: number;
  listened_seconds: number;
}

export interface HomeLibraryProfile {
  composer_count: number;
  genre_count: number;
  total_duration_seconds: number;
  lossless_tracks: number;
  hi_res_tracks: number;
  hi_res_albums: number;
}

export interface HomeEssentials {
  continue_listening: ResponseSong[];
  recommended: ResponseSong[];
  shuffle: ResponseSong[];
  albums: ResponseAlbum[];
  stats: {
    song_count: number;
    album_count: number;
    artist_count: number;
  };
  /** Discovery rows; absent from servers older than the redesign. */
  profile?: HomeLibraryProfile;
  recently_added?: HomeAlbum[];
  daily_picks?: { date: string; picks: HomeDailyPick[] };
  new_for_you?: HomeAlbum[];
  similar_artists?: HomeArtist[];
  best_years?: HomeBestYears | null;
  genres?: HomeGenre[];
  /** Plays and time listened from history; older servers leave this out. */
  listening?: HomeListening;
}

/** Library artists sharing genres with an artist, for its page. */
export async function getSimilarArtists(id: string): Promise<HomeArtist[]> {
  const res = await api.get<HomeArtist[]>(
    `/artists/${encodeURIComponent(id)}/similar`,
  );
  return res.data;
}

export async function getHomeEssentials(): Promise<HomeEssentials> {
  const res = await api.get("/home");
  return res.data;
}

/**
 * The short quality label for an album badge: "24/96" for high resolution,
 * "16/44.1" for CD-quality lossless, the codec for lossy files; null when the
 * format is unknown.
 */
export function albumQualityLabel(
  quality: HomeAlbumQuality | null | undefined,
): string | null {
  if (!quality) return null;
  if (quality.label) return quality.label;
  if (quality.lossless && quality.bit_depth && quality.sample_rate) {
    const khz = quality.sample_rate / 1000;
    const rate = Number.isInteger(khz) ? String(khz) : khz.toFixed(1);
    return `${quality.bit_depth}/${rate}`;
  }
  return quality.codec ? quality.codec.toUpperCase() : null;
}

/** "Good morning" / "Good afternoon" / "Good evening" for a local hour. */
export function greetingForHour(hour: number): string {
  if (hour >= 5 && hour < 12) return "Good morning";
  if (hour >= 12 && hour < 18) return "Good afternoon";
  return "Good evening";
}

export interface HomeStat {
  label: string;
  value: string;
  unit?: string;
  /** Marks the lossless share with the copper signal dot. */
  accent?: boolean;
}

const number = new Intl.NumberFormat("en-US");

/** "43 min" under an hour, else whole hours ("2,028 h"). */
function durationValue(
  seconds: number,
  allowZero = false,
): { value: string; unit: string } {
  const safe = Math.max(0, seconds);
  if (safe >= 3600) {
    return { value: number.format(Math.round(safe / 3600)), unit: "h" };
  }
  const minutes = Math.round(safe / 60);
  return {
    value: number.format(allowZero ? minutes : Math.max(1, minutes)),
    unit: "min",
  };
}

/**
 * The stat cards under the greeting: Roon's artists / albums / tracks /
 * composers, plus what an audiophile asks next: how many hours, how much is
 * lossless, how many albums are high resolution.
 */
/** Stat cards for Home on any client. */
export function homeStats(feed: HomeEssentials): HomeStat[] {
  const { stats, profile } = feed;
  if (!stats.song_count) return [];
  const cards: HomeStat[] = [
    { label: "Artists", value: number.format(stats.artist_count) },
    { label: "Albums", value: number.format(stats.album_count) },
    { label: "Tracks", value: number.format(stats.song_count) },
  ];
  if (!profile) return cards;
  cards.push({
    label: "Composers",
    value: number.format(profile.composer_count),
  });
  // The library's own length, labelled as such: it is not time listened.
  cards.push({
    label: "Library length",
    ...durationValue(profile.total_duration_seconds),
  });
  if (feed.listening) {
    cards.push({
      label: "You've listened",
      ...durationValue(feed.listening.listened_seconds, true),
    });
  }
  const lossless = Math.round(
    (profile.lossless_tracks / Math.max(1, stats.song_count)) * 100,
  );
  cards.push({
    label: profile.hi_res_albums
      ? `Lossless · ${profile.hi_res_albums} hi-res`
      : "Lossless",
    value: String(lossless),
    unit: "%",
    accent: lossless > 0,
  });
  return cards;
}
