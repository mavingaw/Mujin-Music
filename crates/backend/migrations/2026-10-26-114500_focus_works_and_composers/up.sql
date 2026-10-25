-- Work, movement and release type tags read by tag parser version 18, and
-- composer bookmarks for the Focus browsers.
ALTER TABLE raw_file_metadata ADD COLUMN work TEXT;
ALTER TABLE raw_file_metadata ADD COLUMN movement TEXT;
ALTER TABLE raw_file_metadata ADD COLUMN movement_number INTEGER;
ALTER TABLE raw_file_metadata ADD COLUMN release_type TEXT;

CREATE INDEX idx_raw_file_metadata_release_type ON raw_file_metadata(release_type);
CREATE INDEX idx_raw_file_metadata_work ON raw_file_metadata(work);
CREATE INDEX idx_track_artist_role_artist ON track_artist(role, artist_id, track_id);

-- Saved focuses may now bookmark the Composers browser.
DROP TRIGGER user_data_changed_saved_focus_insert;
DROP TRIGGER user_data_changed_saved_focus_update;
DROP TRIGGER user_data_changed_saved_focus_delete;
DROP INDEX idx_saved_focus_user_position;
ALTER TABLE saved_focus RENAME TO saved_focus_previous;
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
