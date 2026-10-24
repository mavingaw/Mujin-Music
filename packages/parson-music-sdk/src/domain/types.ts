export type User = {
  id: number;
  name?: string;
  username: string;
  image?: string;
  bitrate: number;
  created_at: string;
  updated_at: string;
  now_playing?: string;
  role: "admin" | "user";
};

export type ListenHistoryItem = {
  id: number;
  user_id: number;
  song_id: string;
  listened_at: string;
};

export interface Artist {
  id: string;
  name: string;
  icon_url: string;
  followers: number;
  albums: Album[];
  discography?: ArtistDiscographySection[];
  featured_on_album_ids: string[];
  description: string;
}

export interface ArtistDiscographySection {
  key: string;
  title: string;
  albums: Album[];
}

export interface Album {
  id: string;
  name: string;
  cover_url: string;
  songs: LibrarySong[];
  first_release_date: string;
  musicbrainz_id: string;
  wikidata_id: string | null;
  primary_type: string;
  description: string;
  contributing_artists: string[];
  contributing_artists_ids: string[];
  release_album?: ReleaseAlbum;
  release_group_album?: ReleaseGroupAlbum;
  /** Most common record label tagged on the album's tracks (full album responses). */
  label?: string | null;
  /** Contributor credits merged across the album's tracks (full album responses). */
  credits?: AlbumCredit[];
}

/** Technical properties of a track's primary file, read from its stream headers. */
export interface AudioProperties {
  /** e.g. `flac`, `alac`, `aac`, `mp3`, `vorbis`, `opus`, `pcm`, `pcm_float`. */
  codec: string | null;
  /** e.g. `flac`, `mp4`, `mp3`, `ogg`, `wav`, `aiff`. */
  container: string | null;
  /** Hz. */
  sample_rate: number | null;
  /** Bits per sample, for lossless and PCM codecs only. */
  bit_depth: number | null;
  channels: number | null;
  /** Average audio bitrate in kbps. */
  bitrate_kbps: number | null;
  /** `null` when the codec is unknown. */
  lossless: boolean | null;
}

/** Canonical contributor roles the indexer reads from tags. */
export type CreditRole =
  | "composer"
  | "lyricist"
  | "arranger"
  | "conductor"
  | "performer"
  | "ensemble"
  | "producer"
  | "remixer"
  | "mixer"
  | "engineer";

/** A contributor credit on a track (the primary and featured artists are not repeated here). */
export interface Credit {
  artist_id: string;
  name: string;
  role: CreditRole | (string & {});
  /** Lower-case instrument for `performer` credits, when tagged. */
  instrument: string | null;
}

/** A credit on an album, with the tracks it applies to. */
export interface AlbumCredit extends Credit {
  track_ids: string[];
}

/** Fields the server adds to songs once the library has been indexed with parser version 17. */
export interface SongDetails {
  audio?: AudioProperties | null;
  credits?: Credit[];
  isrc?: string | null;
  label?: string | null;
}

export interface LibrarySong extends SongDetails {
  id: string;
  album_id?: string;
  artist_id?: string;
  name: string;
  artist: string;
  contributing_artists: string[];
  contributing_artist_ids: string[];
  track_number: number;
  path: string;
  duration: number;
  artist_object: Artist;
  album_object: Album;
}

export type ResponseAlbum = Album & { artist_object: Artist };
export type ResponseSong = LibrarySong;

export interface BareSong extends SongDetails {
  id: string;
  name: string;
  artist: string;
  contributing_artists: string[];
  contributing_artist_ids: string[];
  track_number: number;
  path: string;
  duration: number;
}

export type SongMetadataPatch = Partial<
  Pick<
    BareSong,
    | "name"
    | "artist"
    | "contributing_artists"
    | "contributing_artist_ids"
    | "track_number"
    | "path"
    | "duration"
  >
>;

export type AlbumMetadataPatch = Partial<
  Pick<
    Album,
    | "name"
    | "cover_url"
    | "first_release_date"
    | "musicbrainz_id"
    | "wikidata_id"
    | "primary_type"
    | "description"
    | "contributing_artists"
    | "contributing_artists_ids"
  >
>;

export type ArtistMetadataPatch = Partial<
  Pick<
    Artist,
    "name" | "icon_url" | "followers" | "description" | "featured_on_album_ids"
  >
>;

export interface LibraryMetadataPatch {
  song?: SongMetadataPatch;
  album?: AlbumMetadataPatch;
  artist?: ArtistMetadataPatch;
}

export interface LibraryMetadataResponse {
  song: LibrarySong;
  album: Album & { artist_object: Artist };
  artist: Artist;
}

export interface AlbumMetadataResponse {
  album: Album & { artist_object: Artist };
  artist: Artist;
}

export interface ArtistInfo {
  id: string;
  name: string;
  icon_url?: string;
  followers?: number;
}

export interface AlbumInfo {
  id: string;
  name: string;
  cover_url?: string;
  first_release_date?: string;
  primary_type?: string;
}

export interface SongInfo {
  id: string;
  name: string;
  artist?: string;
  contributing_artists?: string[];
  duration: number;
  path: string;
}

export interface CombinedItem {
  item_type: string;
  name: string;
  id: string;
  description?: string;
  acronym?: string;
  artist_object?: ArtistInfo;
  album_object?: AlbumInfo;
  song_object?: SongInfo;
}

export interface ReleaseGroupAlbum {
  cover_url?: string;
  rating: Rating;
  artist_credit: CreditArtist[];
  relationships: Relationship[];
  releases: Information[];
  musicbrainz_id: string;
  first_release_date: string;
  title: string;
  aliases: Alias[];
  primary_type_id: string;
  annotation: string;
  tags: Tag[];
  genres: Genre[];
}

export interface ReleaseAlbum {
  cover_url?: string;
  information: Information;
  tracks: Track[];
  labels: Label[];
  relationships: Relationship[];
  musicbrainz_id: string;
  first_release_date: string;
  title: string;
  aliases: Alias[];
  primary_type_id: string;
  annotation: string;
  tags: Tag[];
  genres: Genre[];
}

export interface Information {
  date: string;
  country: string;
  status_id: string;
  title: string;
  barcode: string;
  quality: string;
  packaging: string;
  disambiguation: string;
  release_type: string;
  asin: string;
  music_brainz_id: string;
  packaging_id: string;
  status: string;
  tags: Tag[];
  genres: Genre[];
  cover_art_status: CoverArtStatus;
  collections: Collection[];
  artist_credits: CreditArtist[];
}

export interface CoverArtStatus {
  count: number;
  front: string;
  darkened: string;
  artwork: string;
  back: string;
}

export interface CreditArtist {
  name: string;
  join_phrase: string;
  musicbrainz_id: string;
  artist_type: string;
  disambiguation: string;
  genres: Genre[];
  aliases: Alias[];
}

export interface Genre {
  musicbrainz_id: string;
  disambiguation: string;
  name: string;
  count: number;
}

export interface Alias {
  begin: string;
  alias_type: string;
  sort_name: string;
  name: string;
  end: string;
  locale: string;
  ended: boolean;
  type_id: string;
  primary: string;
}

export interface Collection {
  entity_type: string;
  type_id: string;
  name: string;
  editor: string;
  release_count: number;
  id: string;
  collection_type: string;
  secondary_type_ids: string[];
  tags: Tag[];
  artist_credit: CreditArtist[];
  aliases: string[];
  secondary_types: string[];
  disambiguation: string;
  first_release_date: string;
}

export interface Track {
  length: number;
  artist_credit: CreditArtist[];
  track_name: string;
  position: number;
  video: boolean;
  first_release_date: string;
  number: string;
  musicbrainz_id: string;
  rating: Rating;
  tags: Tag[];
}

export interface Rating {
  votes_count: number;
  value: number;
}

export interface Tag {
  count: number;
  name: string;
}

export interface Label {
  catalog_number: string;
  type_id: string;
  name: string;
  sort_name: string;
  label_type: string;
  id: string;
  aliases: Alias[];
}

export interface Relationship {
  direction: string;
  type_id: string;
  ended: boolean;
  begin: string;
  purchase_relationship_type: string;
  musicbrainz_id: string;
  target_credit: string;
  source_credit: string;
  target_type: string;
  end: string;
  url: string;
}
