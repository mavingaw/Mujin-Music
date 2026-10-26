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
  /** Other names the sources credit this person under ("Jonathan Davis"
   * for Q-Tip). Older servers leave this out. */
  credited_as?: string[];
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
  | "musicbrainz"
  | "wikidata"
  | "wikipedia"
  | "commons"
  | "relations"
  | "coverart";

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
  /** Canonical genres from tags, MusicBrainz or an edit; older servers leave this out. */
  genres?: GenreRef[];
  /** Whether an administrator chose the genres by hand. */
  genres_edited?: boolean;
  /**
   * About the pieces on the album, from their own Wikipedia articles (what
   * a classical album shows when the release has no article). Older
   * servers leave this out.
   */
  work_texts?: WorkText[];
  /** The match check behind the MusicBrainz release; older servers leave this out. */
  release_match?: ReleaseMatchCheck | null;
}

/** A piece on an album and its Wikipedia About text. */
export interface WorkText {
  work_mbid: string;
  title: string;
  text: EditorialText;
}

/** Where a genre came from. */
export type GenreOrigin = "tags" | "musicbrainz" | "user";

/** Which genres albums and artists show; each falls back to the other. */
export type GenreSource = "online" | "tags";

/** A genre on an album or artist, normalised into the canonical tree. */
export interface GenreRef {
  /** Canonical slug, or a slug made from the name for unknown genres. */
  slug: string;
  name: string;
  /** Ancestor names from the top-level genre down ("Jazz" for Hard Bop). */
  path: string[];
  /** Whether the genre is part of the canonical tree. */
  canonical: boolean;
  sources: (GenreOrigin | (string & {}))[];
  /** The raw names that were normalised to this genre. */
  raw_names: string[];
}

export interface GenreLink {
  slug: string;
  name: string;
}

export interface GenreCount extends GenreLink {
  album_count: number;
}

export interface GenreTreeNode extends GenreCount {
  children: GenreTreeNode[];
}

/** The library's genres as a tree, with album counts. */
export interface GenreTree {
  genres: GenreTreeNode[];
  /** Genres the canonical tree does not know. */
  other: GenreCount[];
  source: GenreSource;
  /** Every canonical genre, for choosing genres when editing an album. */
  catalog?: { slug: string; name: string; parent: string | null }[];
}

export interface GenreAlbum {
  id: string;
  name: string;
  cover_url: string;
  year: string | null;
  artist_id: string;
  artist_name: string;
  genres: GenreLink[];
}

export interface GenrePerson {
  artist_id: string;
  name: string;
  /** The library's artist picture. */
  icon_url: string | null;
  /** API path of a cached Wikimedia Commons photo. */
  image_url: string | null;
  album_count: number;
  track_count: number;
}

/** A genre page: About text, sub-genres and the library's music in it. */
export interface GenreDetails {
  slug: string;
  name: string;
  canonical: boolean;
  path: GenreLink[];
  children: GenreCount[];
  related: GenreCount[];
  /** The Wikipedia article lead, with CC BY-SA attribution. */
  about: EditorialText | null;
  links: ExternalLink[];
  enrichment: EnrichmentStatus;
  album_count: number;
  albums: GenreAlbum[];
  artists: GenrePerson[];
  composers: GenrePerson[];
  source: GenreSource;
}

export interface AlbumGenres {
  album_id: string;
  genres: GenreRef[];
  edited: boolean;
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
  /** The person's canonical id: a page opened under another of their ids
   * (a second spelling, their MusicBrainz id) answers for this one. */
  artist_id: string;
  /** The id the page was opened with. Older servers leave this out. */
  requested_id?: string;
  /** Other names the person is credited under in this library. */
  credited_as?: string[];
  name: string;
  biography: EditorialText | null;
  image: EditorialImage | null;
  facts: ArtistFacts | null;
  links: ExternalLink[];
  /** Contributor roles across the library (composer on 12 tracks, ...). */
  roles: CreditRoleSummary[];
  note: UserNote | null;
  enrichment: EnrichmentStatus;
  /** The artist's genres, or those of the albums they are credited on. */
  genres?: GenreRef[];
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
  /** Genres from MusicBrainz or from file tags; older servers leave this out. */
  genre_source?: GenreSource;
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
  /** The recordings the Focus selection keeps (all without one). */
  recordings: WorkRecording[];
  /** Focus chips over every recording. Older servers leave this out. */
  focus?: FocusSummary;
  /** The work on MusicBrainz. */
  url: string;
  /**
   * About the work from the Wikipedia article MusicBrainz links (a
   * movement shows its piece's text). Older servers leave this out.
   */
  about?: EditorialText | null;
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
  /**
   * The release's track count is known and differs from the album's.
   * Choosing it pairs tracks by title, so some may get the wrong credits or
   * piece. Older servers leave this out.
   */
  track_count_differs?: boolean;
  /** How well the release fits the album's files. Older servers leave this out. */
  fit?: "likely" | "possible" | "unlikely";
  /** The release automatic matching found and rejected. */
  suggested?: boolean;
  url: string;
}

/** One piece of evidence for or against a MusicBrainz release. */
export interface ReleaseMatchSignal {
  key:
    | "tracks"
    | "lengths"
    | "year"
    | "label"
    | "conductor"
    | "ensemble"
    | (string & {});
  outcome: "match" | "partial" | "mismatch" | "unknown";
  points: number;
  /** A sentence a person can check. */
  detail: string;
}

/**
 * The match check behind an album's MusicBrainz release. A release found
 * automatically is only applied when it passes; an `unverified` one is a
 * suggestion, and the album keeps its file-tag data.
 */
export interface ReleaseMatchCheck {
  status: "verified" | "unverified";
  method: "automatic" | "tagged" | "identified";
  release_mbid: string;
  release_title: string;
  release_artist: string;
  url: string;
  score: number;
  threshold: number;
  /** Whether the evidence passed (a tagged or chosen release is used either way). */
  passed?: boolean;
  release_date?: string | null;
  first_release_date?: string | null;
  release_type?: string | null;
  release_track_count?: number;
  local_track_count?: number;
  signals?: ReleaseMatchSignal[];
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

/** A Focus facet group. */
export type FocusGroupKey =
  | "role"
  | "performer"
  | "conductor"
  | "ensemble"
  | "soloist"
  | "instrument"
  | "composer"
  | "form"
  | "period"
  | "quality"
  | "bit_depth"
  | "sample_rate"
  | "source";

/** One value of a Focus group, with how many albums choosing it shows. */
export interface FocusChip {
  /** A person's id, `name:<name>` for someone with no page, or a value
   * such as `24`, `96000`, `hires`, `local`, `symphony`. */
  key: string;
  label: string;
  artist_id?: string | null;
  count: number;
  selected: boolean;
}

export interface FocusGroup {
  key: FocusGroupKey;
  label: string;
  values: FocusChip[];
}

export interface FocusSummary {
  /** Albums or recordings before filtering. */
  total: number;
  /** Albums or recordings the selection keeps. */
  matched: number;
  /** Groups with a value that narrows or is chosen. */
  groups: FocusGroup[];
  /** Chosen values, keyed `group=value`, including any nothing has. */
  selected: FocusChip[];
}

/** An album on a person's discography. */
export interface FocusAlbum {
  album: Album;
  artist: Artist;
  year: number | null;
  /** The person's roles: `main`, `featured`, or a credit role. */
  roles: string[];
  conductors: PersonRef[];
  ensembles: PersonRef[];
  /** The best format on the album, e.g. "FLAC 24-bit / 96 kHz". */
  format: string | null;
  quality: "hires" | "lossless" | "lossy" | null;
  sources: string[];
}

/** Everything a person made, appears on, conducted, composed or is
 * credited on, filtered by a Focus selection. */
export interface PersonDiscography {
  /** The person's canonical id. */
  artist_id: string;
  requested_id: string;
  name: string;
  /** Other names the person is credited under. */
  credited_as: string[];
  albums: FocusAlbum[];
  focus: FocusSummary;
}

/** An album two related people share. */
export interface RelatedAlbum {
  id: string;
  name: string;
  cover_url: string;
}

/** A person related to another through the music in the library. */
export interface RelatedPerson {
  /** Their page, or null when only a name is known. */
  artist_id: string | null;
  name: string;
  /** Their stored photo (an API path), when there is one. */
  image_url: string | null;
  /** Instruments and roles on the shared music, most frequent first. */
  roles: string[];
  /** For top performers: what they are on the recordings. */
  kind?: "conductor" | "ensemble" | "soloist" | "artist";
  album_count: number;
  track_count: number;
  /** Distinct works (or tracks, without work links) connecting them. */
  work_count: number;
  /** Up to three shared albums. */
  albums: RelatedAlbum[];
}

/** Someone Wikidata relates to the person by influence. */
export interface InfluencePerson {
  artist_id: string | null;
  name: string;
  description: string | null;
  image_url: string | null;
  /** Whether they have music in this library. */
  in_library: boolean;
  wikidata_url: string;
}

/** The related-people graph on a person page. */
export interface PersonRelations {
  artist_id: string;
  requested_id: string;
  /** People who made music with the person, by shared releases. */
  collaborators: RelatedPerson[];
  /** Conductors, ensembles and soloists on the most recordings of the
   * person's works. */
  top_performers: RelatedPerson[];
  /** Composers whose works the person records. */
  performing_the_music_of: RelatedPerson[];
  /** Who influenced the person (Wikidata P737). */
  influences: InfluencePerson[];
  /** Whom the person influenced. */
  followers: InfluencePerson[];
  influence_source: {
    source: string;
    url: string;
    license: string;
    license_url: string;
  } | null;
  /** Whether online metadata, and so influences, is on. */
  enrichment_enabled: boolean;
}
