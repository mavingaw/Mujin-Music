-- Library tools: maintenance state that never touches the user's files.
-- Every row here is a choice or a finding layered over the catalog; the
-- files on disk are only ever read.

-- Files the scanner saw but did not import, with the exact reason. Filled
-- by the skipped-files check; `ignored` hides a row the user dismissed.
CREATE TABLE library_skipped_file(
  path TEXT NOT NULL PRIMARY KEY,
  root_path TEXT NOT NULL,
  reason TEXT NOT NULL,
  detail TEXT NOT NULL DEFAULT '',
  extension TEXT NOT NULL DEFAULT '',
  size_bytes INTEGER NOT NULL DEFAULT 0,
  modified_at_ns INTEGER NOT NULL DEFAULT 0,
  ignored INTEGER NOT NULL DEFAULT 0,
  first_seen_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  checked_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_library_skipped_file_reason ON library_skipped_file(reason, ignored);

-- Per-file visibility: a copy hidden from the library, or merged into the
-- copy the user keeps (likes and playlist entries move to the kept copy).
CREATE TABLE library_file_visibility(
  path TEXT NOT NULL PRIMARY KEY,
  state TEXT NOT NULL CHECK(state IN ('hidden', 'merged')),
  kept_path TEXT,
  song_id TEXT NOT NULL DEFAULT '',
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Which file plays when one track has several copies (FLAC and MP3 of the
-- same recording). Without a row the highest quality copy plays.
CREATE TABLE library_file_preference(
  track_id TEXT NOT NULL PRIMARY KEY,
  path TEXT NOT NULL,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Duplicate groups the user marked as "not duplicates". The key is a hash
-- of the member paths, so a new copy brings the group back for review.
CREATE TABLE library_duplicate_dismissal(
  group_key TEXT NOT NULL PRIMARY KEY,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Album versions: manual grouping (group_key NULL keeps an album out of any
-- automatic group) and the preferred version of each group.
CREATE TABLE library_album_version_member(
  album_id TEXT NOT NULL PRIMARY KEY,
  group_key TEXT,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE library_album_version_choice(
  group_key TEXT NOT NULL PRIMARY KEY,
  preferred_album_id TEXT NOT NULL,
  hide_others INTEGER NOT NULL DEFAULT 0,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Albumizer: albums the user made from loose tracks, and which file goes
-- where. Tracks are keyed by path so a rescan keeps the grouping.
CREATE TABLE library_user_album(
  id TEXT NOT NULL PRIMARY KEY,
  title TEXT NOT NULL,
  artist_id TEXT,
  artist_name TEXT NOT NULL,
  year TEXT NOT NULL DEFAULT '',
  primary_type TEXT NOT NULL DEFAULT 'album',
  cover_path TEXT NOT NULL DEFAULT '',
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE library_user_album_track(
  path TEXT NOT NULL PRIMARY KEY,
  album_id TEXT NOT NULL,
  position INTEGER NOT NULL,
  disc INTEGER NOT NULL DEFAULT 1,
  FOREIGN KEY(album_id) REFERENCES library_user_album(id) ON DELETE CASCADE
);
CREATE INDEX idx_library_user_album_track_album ON library_user_album_track(album_id, position);

-- Audio analysis per file version (size and modification time): a peak
-- waveform for display and an acoustic fingerprint for duplicate checks.
CREATE TABLE library_audio_analysis(
  path TEXT NOT NULL PRIMARY KEY,
  file_version TEXT NOT NULL,
  waveform TEXT NOT NULL DEFAULT '',
  fingerprint TEXT NOT NULL DEFAULT '',
  fingerprint_kind TEXT NOT NULL DEFAULT '',
  duration_seconds DOUBLE NOT NULL DEFAULT 0,
  error TEXT,
  analyzed_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
