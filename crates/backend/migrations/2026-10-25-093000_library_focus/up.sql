-- Library Focus: user tags, saved focuses (bookmarks), smart playlists and
-- the indexes the facet query builder in src/library/focus.rs relies on.

CREATE TABLE user_tag (
  id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER NOT NULL,
  name TEXT NOT NULL,
  normalized_name TEXT NOT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT user_tag_user_fkey FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE,
  CONSTRAINT user_tag_name_key UNIQUE (user_id, normalized_name)
);

CREATE TABLE user_tag_item (
  tag_id INTEGER NOT NULL,
  entity_type TEXT NOT NULL CHECK (entity_type IN ('track', 'album', 'artist')),
  entity_id TEXT NOT NULL,
  added_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (tag_id, entity_type, entity_id),
  CONSTRAINT user_tag_item_tag_fkey FOREIGN KEY (tag_id) REFERENCES user_tag(id) ON DELETE CASCADE
) WITHOUT ROWID;

CREATE INDEX idx_user_tag_item_entity ON user_tag_item(entity_type, entity_id, tag_id);

CREATE TABLE saved_focus (
  id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER NOT NULL,
  name TEXT NOT NULL,
  view TEXT NOT NULL CHECK (view IN ('albums', 'songs', 'artists')),
  query_json TEXT NOT NULL,
  position INTEGER NOT NULL DEFAULT 0,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT saved_focus_user_fkey FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
);

CREATE INDEX idx_saved_focus_user_position ON saved_focus(user_id, position, id);

CREATE TABLE smart_playlist (
  playlist_id INTEGER NOT NULL PRIMARY KEY,
  query_json TEXT NOT NULL,
  track_limit INTEGER NOT NULL,
  refreshed_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT smart_playlist_playlist_fkey FOREIGN KEY (playlist_id) REFERENCES playlist(id) ON DELETE CASCADE
);

CREATE TRIGGER user_data_changed_user_tag_insert AFTER INSERT ON user_tag BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;
CREATE TRIGGER user_data_changed_user_tag_update AFTER UPDATE ON user_tag BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;
CREATE TRIGGER user_data_changed_user_tag_delete AFTER DELETE ON user_tag BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;
CREATE TRIGGER user_data_changed_user_tag_item_insert AFTER INSERT ON user_tag_item BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;
CREATE TRIGGER user_data_changed_user_tag_item_delete AFTER DELETE ON user_tag_item BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;
CREATE TRIGGER user_data_changed_saved_focus_insert AFTER INSERT ON saved_focus BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;
CREATE TRIGGER user_data_changed_saved_focus_update AFTER UPDATE ON saved_focus BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;
CREATE TRIGGER user_data_changed_saved_focus_delete AFTER DELETE ON saved_focus BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;
CREATE TRIGGER user_data_changed_smart_playlist_insert AFTER INSERT ON smart_playlist BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;
CREATE TRIGGER user_data_changed_smart_playlist_update AFTER UPDATE ON smart_playlist BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;
CREATE TRIGGER user_data_changed_smart_playlist_delete AFTER DELETE ON smart_playlist BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;

-- Facet filters start from a value and walk to tracks, so each join table
-- needs an index that leads with the filtered column.
CREATE INDEX idx_track_genre_genre ON track_genre(genre_id, track_id);
CREATE INDEX idx_album_genre_genre ON album_genre(genre_id, album_id);
CREATE INDEX idx_track_artist_artist_role ON track_artist(artist_id, role, track_id);
CREATE INDEX idx_track_artist_track_role ON track_artist(track_id, role, artist_id);
CREATE INDEX idx_album_artist_album_role ON album_artist(album_id, role, artist_id);
CREATE INDEX idx_track_file_file ON track_file(file_id, track_id);
CREATE INDEX idx_raw_file_metadata_codec ON raw_file_metadata(codec);
CREATE INDEX idx_raw_file_metadata_sample_rate ON raw_file_metadata(sample_rate);
CREATE INDEX idx_raw_file_metadata_bit_depth ON raw_file_metadata(bit_depth);
CREATE INDEX idx_raw_file_metadata_label ON raw_file_metadata(label);
CREATE INDEX idx_album_entity_release_year
  ON album_entity(CAST(substr(first_release_date, 1, 4) AS INTEGER));
CREATE INDEX idx_album_entity_primary_type ON album_entity(primary_type);
CREATE INDEX idx_album_entity_created ON album_entity(created_at);
CREATE INDEX idx_track_entity_created ON track_entity(created_at);
CREATE INDEX idx_listen_history_user_song ON listen_history_item(user_id, song_id);
