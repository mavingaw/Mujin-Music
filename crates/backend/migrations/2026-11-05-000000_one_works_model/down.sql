-- Splits the one works model back into the library's tag works and the
-- MusicBrainz works. The library tables are rebuilt from the catalog.
DROP VIEW library_track_work;
DROP VIEW library_work;

CREATE TABLE musicbrainz_work (
  mbid TEXT NOT NULL PRIMARY KEY,
  title TEXT,
  work_type TEXT,
  composed_year INTEGER,
  parent_mbid TEXT,
  composer_mbids_json TEXT NOT NULL DEFAULT '[]',
  status TEXT NOT NULL,
  fetched_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE musicbrainz_composer (
  mbid TEXT NOT NULL PRIMARY KEY,
  name TEXT,
  normalized_name TEXT,
  birth_year INTEGER,
  death_year INTEGER,
  country TEXT,
  status TEXT NOT NULL,
  fetched_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_musicbrainz_composer_name ON musicbrainz_composer(normalized_name);

CREATE TABLE work_entity (
  id TEXT NOT NULL PRIMARY KEY,
  title TEXT NOT NULL,
  musicbrainz_work_id TEXT,
  form TEXT,
  instrumentation TEXT,
  scoring TEXT,
  composed_year INTEGER,
  period TEXT,
  tagged INTEGER NOT NULL DEFAULT 0,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_work_entity_form ON work_entity(form, id);
CREATE INDEX idx_work_entity_instrumentation ON work_entity(instrumentation, id);
CREATE INDEX idx_work_entity_period ON work_entity(period, id);
CREATE INDEX idx_work_entity_composed_year ON work_entity(composed_year, id);

CREATE TABLE editorial_track_work(
  track_id TEXT NOT NULL,
  album_id TEXT NOT NULL,
  work_mbid TEXT NOT NULL,
  recording_mbid TEXT,
  source TEXT NOT NULL DEFAULT 'musicbrainz',
  PRIMARY KEY(track_id, work_mbid)
);
CREATE INDEX idx_editorial_track_work_work ON editorial_track_work(work_mbid);
CREATE INDEX idx_editorial_track_work_album ON editorial_track_work(album_id);
INSERT INTO editorial_track_work (track_id, album_id, work_mbid, recording_mbid, source)
SELECT track_id, album_id, work_mbid, recording_mbid, source FROM track_work
WHERE source = 'musicbrainz' ON CONFLICT DO NOTHING;

DROP TABLE track_work;
CREATE TABLE track_work (
  track_id TEXT NOT NULL PRIMARY KEY,
  work_id TEXT NOT NULL,
  movement TEXT,
  movement_number INTEGER
);
CREATE INDEX idx_track_work_work ON track_work(work_id, track_id);

DROP INDEX idx_work_composer_artist_work;
DELETE FROM work_composer;
CREATE INDEX idx_work_composer_artist ON work_composer(artist_id, work_id);
ALTER TABLE composer_entity DROP COLUMN country;
DELETE FROM composer_entity;

DELETE FROM work WHERE mbid LIKE 'wk\_%' ESCAPE '\';
DROP INDEX idx_work_library_period;
DROP INDEX idx_work_library_form;
ALTER TABLE work DROP COLUMN musicbrainz_facts;
ALTER TABLE work DROP COLUMN created_at;
ALTER TABLE work DROP COLUMN in_library;
ALTER TABLE work DROP COLUMN tagged;
ALTER TABLE work DROP COLUMN period;
ALTER TABLE work DROP COLUMN composed_year;
ALTER TABLE work DROP COLUMN scoring;
ALTER TABLE work DROP COLUMN instrumentation;

DELETE FROM work_catalog_state;
