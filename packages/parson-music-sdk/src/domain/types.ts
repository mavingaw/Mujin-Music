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

/** Where a credit came from: file tags or an external source such as MusicBrainz. */
export type CreditSource = "tags" | "musicbrainz" | (string & {});

/**
 * A credit on an album page, merged across tags and external sources.
 * `artist_id` is a local artist id, `mb:<MusicBrainz id>` for a person only
 * known from MusicBrainz, or `null` when the person cannot be browsed.
 * An empty `track_ids` list means the credit applies to the whole release.
 */
export interface AboutCredit {
  artist_id: string | null;
  name: string;
  role: CreditRole | (string & {});
  instrument: string | null;
  track_ids: string[];
  sources: CreditSource[];
  /** The person's cached photo, relative to the API base
   * (`/artists/{id}/image`), when there is one. */
  image_url?: string | null;
}

/** Licensed text (Wikipedia, CC BY-SA 4.0) stored with its attribution. */
export interface EditorialText {
  kind: "biography" | "background" | "reception";
  source: "wikipedia" | (string & {});
  language: string;
  /** Article title. */
  title: string;
  body: string;
  /** Article URL. */
  url: string;
  license: string;
  license_url: string;
}

export interface EditorialImage {
  /** Server path relative to the API base, e.g. `/artists/{id}/image`. */
  url: string;
  author: string;
  license: string;
  license_url: string | null;
  page_url: string;
}

export interface ExternalLink {
  provider: "musicbrainz" | "wikidata" | (string & {});
  url: string;
}

/** A private note the signed-in user wrote about an album or artist. */
export interface UserNote {
  body: string;
  updated_at: string;
}

/** The lookup stages; each has its own queue and back-off. */
export type EnrichmentStage =
  "musicbrainz" | "wikidata" | "wikipedia" | "commons";

export interface EnrichmentTaskState {
  status: "pending" | "done" | "skipped" | "failed" | (string & {});
  last_error: string | null;
  updated_at: string;
  /** The stage still at work, so a page can say what it waits for. */
  stage?: EnrichmentStage | (string & {});
}

export interface EnrichmentStageStatus {
  stage: EnrichmentStage;
  host: string;
  counts: { pending: number; done: number; skipped: number; failed: number };
  /** Seconds until a busy host is asked again; 0 when it is not paused. */
  paused_seconds: number;
}

export interface EnrichmentStatus {
  /** Whether an administrator turned on online metadata enrichment. */
  enabled: boolean;
  task: EnrichmentTaskState | null;
}

export interface AlbumReleaseFacts {
  /**
   * Whether the matched MusicBrainz release is known to be this exact
   * edition (tagged id or identical track list). Edition facts such as
   * `date`, `country`, `barcode` and catalog numbers only apply when true.
   */
  exact?: boolean;
  date?: string | null;
  first_release_date?: string | null;
  country?: string | null;
  barcode?: string | null;
  release_type?: string | null;
  labels?: { name: string; catalog_number: string | null }[];
}

export interface AlbumAbout {
  album_id: string;
  credits: AboutCredit[];
  /** The Wikipedia article lead about the album. */
  background: EditorialText | null;
  /** The Wikipedia article's critical reception section. */
  review: EditorialText | null;
  facts: AlbumReleaseFacts | null;
  links: ExternalLink[];
  note: UserNote | null;
  enrichment: EnrichmentStatus;
  /** The work (and the piece it belongs to) each track performs. */
  works?: TrackWork[];
  /** The release an administrator chose with "Identify album". */
  identification?: AlbumIdentification | null;
}

export interface ArtistFacts {
  name?: string | null;
  /** `Person`, `Group`, `Orchestra`, `Choir`, ... */
  kind?: string | null;
  area?: string | null;
  begin_area?: string | null;
  /** Born or formed. */
  begin?: string | null;
  /** Died or disbanded. */
  end?: string | null;
  disambiguation?: string | null;
}

export interface CreditRoleSummary {
  role: CreditRole | (string & {});
  instrument: string | null;
  track_count: number;
  album_count: number;
}

export interface ArtistAbout {
  artist_id: string;
  name: string;
  biography: EditorialText | null;
  image: EditorialImage | null;
  facts: ArtistFacts | null;
  links: ExternalLink[];
  /** Contributor roles across the library (composer on 12 tracks, ...). */
  roles: CreditRoleSummary[];
  note: UserNote | null;
  enrichment: EnrichmentStatus;
}

export interface CreditRoleRef {
  role: CreditRole | (string & {});
  instrument: string | null;
}

export interface CreditedAlbum {
  album: Album;
  artist: Artist;
  roles: CreditRoleRef[];
  /** Credited tracks on the album; empty for a release-wide credit. */
  track_ids: string[];
}

export interface CreditedSong {
  song: LibrarySong;
  roles: CreditRoleRef[];
}

/** Everything a person is credited on, for linked credit browsing. */
export interface ArtistCredits {
  artist_id: string;
  name: string;
  /** Whether the person also has their own albums in the library. */
  has_discography: boolean;
  roles: CreditRoleSummary[];
  albums: CreditedAlbum[];
  songs: CreditedSong[];
}

export interface EnrichmentSettings {
  enabled: boolean;
  language: string;
  artist_images: boolean;
}

export interface EnrichmentOverview extends EnrichmentSettings {
  counts: { pending: number; done: number; skipped: number; failed: number };
  /** Hosts enrichment contacts when enabled. */
  sources: string[];
  /** Each stage's queue; older servers leave this out. */
  stages?: EnrichmentStageStatus[];
}

/** A person on a work or recording. `artist_id` is a local artist id,
 * `mb:<MusicBrainz id>` for someone only known from MusicBrainz, or null. */
export interface PersonRef {
  artist_id: string | null;
  name: string;
  instrument?: string | null;
}

/** A composition, keyed by its MusicBrainz work id. */
export interface WorkSummary {
  mbid: string;
  title: string;
  /** Form or MusicBrainz work type: Symphony, Sonata, Song, ... */
  form: string | null;
  key: string | null;
  /** Catalogue number such as "Op. 125" or "BWV 1007". */
  catalogue: string | null;
  /** Year or range composed, e.g. "1822–1824". */
  composed: string | null;
  /** Style period from the composition year: Baroque, Classical, ... */
  period: string | null;
  composer: PersonRef | null;
  language: string | null;
  parent_mbid: string | null;
  part_position: number | null;
}

/** A track's work; `parent` is the piece a movement belongs to. */
export interface TrackWork {
  track_id: string;
  work: WorkSummary;
  parent: WorkSummary | null;
  /** The movement title under the piece's heading, e.g. "IV. Presto". */
  part_title: string;
}

export interface RecordingTrack {
  song: LibrarySong;
  work_mbid: string;
  part_title: string;
  part_position: number | null;
}

/** One album's recording of a work. */
export interface WorkRecording {
  album: Album;
  artist: Artist;
  year: number | null;
  conductors: PersonRef[];
  ensembles: PersonRef[];
  soloists: PersonRef[];
  /** Track artists not already named as conductor, ensemble or soloist. */
  artists: PersonRef[];
  tracks: RecordingTrack[];
  /** Every known movement of the work is on the album. */
  complete: boolean;
  duration: number;
}

export interface WorkPage {
  work: WorkSummary;
  parent: WorkSummary | null;
  /** Movements with at least one recording in the library. */
  parts: WorkSummary[];
  recordings: WorkRecording[];
  /** The work on MusicBrainz. */
  url: string;
}

export interface ComposerWork {
  work: WorkSummary;
  track_count: number;
  recording_count: number;
}

export interface ComposerWorks {
  artist_id: string;
  name: string | null;
  works: ComposerWork[];
}

export interface AlbumIdentification {
  release_mbid: string;
  release_title: string;
  release_artist: string;
  created_at: string;
}

/** A MusicBrainz release offered by "Identify album". */
export interface ReleaseMatch {
  mbid: string;
  title: string;
  artist: string;
  date: string | null;
  country: string | null;
  track_count: number | null;
  labels: string[];
  catalog_numbers: string[];
  format: string | null;
  release_type: string | null;
  disambiguation: string | null;
  score: number;
  same_track_count: boolean;
  url: string;
}

export interface IdentifySearch {
  album_id: string;
  /** The MusicBrainz search that ran. */
  query: string;
  title: string;
  artist: string;
  local_track_count: number;
  identification: AlbumIdentification | null;
  releases: ReleaseMatch[];
}
