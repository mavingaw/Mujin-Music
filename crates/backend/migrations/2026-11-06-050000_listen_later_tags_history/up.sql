-- Listen Later, smart tags and a listening history that keeps what was
-- played, where, and what it was called at the time.

-- Listen Later: albums, tracks and artists a profile saved for later.
CREATE TABLE listen_later_item (
  user_id INTEGER NOT NULL,
  entity_type TEXT NOT NULL CHECK (entity_type IN ('album', 'track', 'artist')),
  entity_id TEXT NOT NULL,
  added_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (user_id, entity_type, entity_id),
  CONSTRAINT listen_later_item_user_fkey FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
) WITHOUT ROWID;

CREATE INDEX idx_listen_later_user_added ON listen_later_item(user_id, added_at DESC);

CREATE TABLE listen_later_setting (
  user_id INTEGER NOT NULL PRIMARY KEY,
  -- Take a track off the list once it has played, and an album once most
  -- of its tracks have.
  remove_when_played INTEGER NOT NULL DEFAULT 0 CHECK (remove_when_played IN (0, 1)),
  CONSTRAINT listen_later_setting_user_fkey FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
);

CREATE TRIGGER user_data_changed_listen_later_insert AFTER INSERT ON listen_later_item BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;
CREATE TRIGGER user_data_changed_listen_later_delete AFTER DELETE ON listen_later_item BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;

-- Smart tags follow a saved Focus; manual tags keep `query_json` NULL.
ALTER TABLE user_tag ADD COLUMN query_json TEXT;
ALTER TABLE user_tag ADD COLUMN color TEXT;
ALTER TABLE user_tag ADD COLUMN description TEXT;

-- Tags apply to works and playlists too.
CREATE TABLE user_tag_item_next (
  tag_id INTEGER NOT NULL,
  entity_type TEXT NOT NULL CHECK (entity_type IN ('track', 'album', 'artist', 'work', 'playlist')),
  entity_id TEXT NOT NULL,
  added_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (tag_id, entity_type, entity_id),
  CONSTRAINT user_tag_item_tag_fkey FOREIGN KEY (tag_id) REFERENCES user_tag(id) ON DELETE CASCADE
) WITHOUT ROWID;
INSERT INTO user_tag_item_next(tag_id, entity_type, entity_id, added_at)
SELECT tag_id, entity_type, entity_id, added_at FROM user_tag_item;
DROP TABLE user_tag_item;
ALTER TABLE user_tag_item_next RENAME TO user_tag_item;
CREATE INDEX idx_user_tag_item_entity ON user_tag_item(entity_type, entity_id, tag_id);
CREATE TRIGGER user_data_changed_user_tag_item_insert AFTER INSERT ON user_tag_item BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;
CREATE TRIGGER user_data_changed_user_tag_item_delete AFTER DELETE ON user_tag_item BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;

-- History: where a play happened and what the track was called then, so
-- history, stats and exports survive files that later leave the library.
ALTER TABLE listen_history_item ADD COLUMN zone_id TEXT;
ALTER TABLE listen_history_item ADD COLUMN zone_name TEXT;
ALTER TABLE listen_history_item ADD COLUMN source TEXT;
ALTER TABLE listen_history_item ADD COLUMN device TEXT;
ALTER TABLE listen_history_item ADD COLUMN title TEXT;
ALTER TABLE listen_history_item ADD COLUMN artist_id TEXT;
ALTER TABLE listen_history_item ADD COLUMN artist_name TEXT;
ALTER TABLE listen_history_item ADD COLUMN album_id TEXT;
ALTER TABLE listen_history_item ADD COLUMN album_name TEXT;
ALTER TABLE listen_history_item ADD COLUMN duration_seconds REAL;

UPDATE listen_history_item SET
  title = (SELECT t.title FROM track_entity t WHERE t.id = listen_history_item.song_id),
  album_id = (SELECT t.album_id FROM track_entity t WHERE t.id = listen_history_item.song_id),
  album_name = (
    SELECT a.title FROM track_entity t JOIN album_entity a ON a.id = t.album_id
    WHERE t.id = listen_history_item.song_id
  ),
  artist_id = (
    SELECT ta.artist_id FROM track_artist ta
    WHERE ta.track_id = listen_history_item.song_id AND ta.role = 'primary'
    ORDER BY ta.position, ta.artist_id LIMIT 1
  ),
  artist_name = (
    SELECT ar.name FROM track_artist ta JOIN artist_entity ar ON ar.id = ta.artist_id
    WHERE ta.track_id = listen_history_item.song_id AND ta.role = 'primary'
    ORDER BY ta.position, ta.artist_id LIMIT 1
  ),
  duration_seconds = (
    SELECT t.duration_seconds FROM track_entity t WHERE t.id = listen_history_item.song_id
  );

CREATE INDEX idx_listen_history_user_zone
  ON listen_history_item(user_id, zone_id, listened_at DESC);
CREATE INDEX idx_listen_history_user_artist
  ON listen_history_item(user_id, artist_id, listened_at DESC);
