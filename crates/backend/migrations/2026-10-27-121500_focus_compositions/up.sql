-- Compositions: works grouped from WORK / MusicBrainz work id tags with
-- form, instrumentation, composition year and period, composer profiles
-- (period, nationality, life dates), the MusicBrainz cache that enriches
-- them, and the Compositions browser for saved focuses.
ALTER TABLE raw_file_metadata ADD COLUMN work_mbid TEXT;
ALTER TABLE raw_file_metadata ADD COLUMN composed_year INTEGER;
CREATE INDEX idx_raw_file_metadata_work_mbid ON raw_file_metadata(work_mbid);

-- Rebuilt from the catalog whenever its fingerprint changes.
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

CREATE TABLE track_work (
  track_id TEXT NOT NULL PRIMARY KEY,
  work_id TEXT NOT NULL,
  movement TEXT,
  movement_number INTEGER
);
CREATE INDEX idx_track_work_work ON track_work(work_id, track_id);

CREATE TABLE work_composer (
  work_id TEXT NOT NULL,
  artist_id TEXT NOT NULL,
  position INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (work_id, artist_id)
);
CREATE INDEX idx_work_composer_artist ON work_composer(artist_id, work_id);

CREATE TABLE composer_entity (
  artist_id TEXT NOT NULL PRIMARY KEY,
  birth_year INTEGER,
  death_year INTEGER,
  nationality TEXT,
  period TEXT,
  source TEXT NOT NULL DEFAULT 'derived'
);
CREATE INDEX idx_composer_entity_period ON composer_entity(period, artist_id);
CREATE INDEX idx_composer_entity_nationality ON composer_entity(nationality, artist_id);

CREATE TABLE work_catalog_state (
  singleton INTEGER NOT NULL PRIMARY KEY CHECK (singleton = 1),
  fingerprint TEXT NOT NULL,
  built_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- MusicBrainz lookups, keyed by MBID. `status` is 'ok' or 'missing'.
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

-- Saved focuses may now bookmark the Compositions browser.
DROP TRIGGER user_data_changed_saved_focus_insert;
DROP TRIGGER user_data_changed_saved_focus_update;
DROP TRIGGER user_data_changed_saved_focus_delete;
DROP INDEX idx_saved_focus_user_position;
ALTER TABLE saved_focus RENAME TO saved_focus_previous;
CREATE TABLE saved_focus (
  id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER NOT NULL,
  name TEXT NOT NULL,
  view TEXT NOT NULL CHECK (view IN ('albums', 'songs', 'artists', 'composers', 'compositions')),
  query_json TEXT NOT NULL,
  position INTEGER NOT NULL DEFAULT 0,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT saved_focus_user_fkey FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
);
INSERT INTO saved_focus (id, user_id, name, view, query_json, position, created_at, updated_at)
  SELECT id, user_id, name, view, query_json, position, created_at, updated_at FROM saved_focus_previous;
DROP TABLE saved_focus_previous;
CREATE INDEX idx_saved_focus_user_position ON saved_focus(user_id, position, id);
CREATE TRIGGER user_data_changed_saved_focus_insert AFTER INSERT ON saved_focus BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;
CREATE TRIGGER user_data_changed_saved_focus_update AFTER UPDATE ON saved_focus BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;
CREATE TRIGGER user_data_changed_saved_focus_delete AFTER DELETE ON saved_focus BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;
