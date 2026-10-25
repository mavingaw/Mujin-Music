-- Works (compositions) keyed by MusicBrainz work id, the tracks that
-- perform them, and albums an administrator identified by hand. Like the
-- other editorial tables these hold catalog ids without foreign keys so a
-- rescan that rebuilds catalog rows keeps them.
CREATE TABLE work(
  mbid TEXT NOT NULL PRIMARY KEY,
  title TEXT NOT NULL,
  -- Musical form or MusicBrainz work type: Symphony, Sonata, Song, ...
  form TEXT,
  musical_key TEXT,
  -- Catalogue number such as "Op. 125" or "BWV 1007".
  catalogue TEXT,
  -- Year or range composed, e.g. "1822–1824".
  composed TEXT,
  composer_name TEXT,
  composer_normalized TEXT,
  composer_mbid TEXT,
  language TEXT,
  -- The larger work a movement belongs to, and its position there.
  parent_mbid TEXT,
  part_position INTEGER,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_work_parent ON work(parent_mbid);
CREATE INDEX idx_work_composer_normalized ON work(composer_normalized);
CREATE INDEX idx_work_composer_mbid ON work(composer_mbid);

CREATE TABLE track_work(
  track_id TEXT NOT NULL,
  album_id TEXT NOT NULL,
  work_mbid TEXT NOT NULL,
  recording_mbid TEXT,
  source TEXT NOT NULL DEFAULT 'musicbrainz',
  PRIMARY KEY(track_id, work_mbid)
);
CREATE INDEX idx_track_work_work ON track_work(work_mbid);
CREATE INDEX idx_track_work_album ON track_work(album_id);

-- A MusicBrainz release chosen with "Identify album". It wins over tags and
-- automatic matching until it is cleared.
CREATE TABLE album_identification(
  album_id TEXT NOT NULL PRIMARY KEY,
  release_mbid TEXT NOT NULL,
  release_title TEXT NOT NULL DEFAULT '',
  release_artist TEXT NOT NULL DEFAULT '',
  identified_by INTEGER,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
