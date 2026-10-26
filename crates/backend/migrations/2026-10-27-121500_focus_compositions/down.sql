DROP TRIGGER user_data_changed_saved_focus_insert;
DROP TRIGGER user_data_changed_saved_focus_update;
DROP TRIGGER user_data_changed_saved_focus_delete;
DROP INDEX idx_saved_focus_user_position;
ALTER TABLE saved_focus RENAME TO saved_focus_next;
CREATE TABLE saved_focus (
  id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER NOT NULL,
  name TEXT NOT NULL,
  view TEXT NOT NULL CHECK (view IN ('albums', 'songs', 'artists', 'composers')),
  query_json TEXT NOT NULL,
  position INTEGER NOT NULL DEFAULT 0,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT saved_focus_user_fkey FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
);
INSERT INTO saved_focus (id, user_id, name, view, query_json, position, created_at, updated_at)
  SELECT id, user_id, name, view, query_json, position, created_at, updated_at
  FROM saved_focus_next WHERE view <> 'compositions';
DROP TABLE saved_focus_next;
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
DROP TABLE musicbrainz_composer;
DROP TABLE musicbrainz_work;
DROP TABLE work_catalog_state;
DROP TABLE composer_entity;
DROP TABLE work_composer;
DROP TABLE track_work;
DROP TABLE work_entity;
DROP INDEX idx_raw_file_metadata_work_mbid;
ALTER TABLE raw_file_metadata DROP COLUMN composed_year;
ALTER TABLE raw_file_metadata DROP COLUMN work_mbid;
