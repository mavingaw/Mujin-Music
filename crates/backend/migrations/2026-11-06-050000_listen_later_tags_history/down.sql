DROP INDEX IF EXISTS idx_listen_history_user_artist;
DROP INDEX IF EXISTS idx_listen_history_user_zone;
ALTER TABLE listen_history_item DROP COLUMN duration_seconds;
ALTER TABLE listen_history_item DROP COLUMN album_name;
ALTER TABLE listen_history_item DROP COLUMN album_id;
ALTER TABLE listen_history_item DROP COLUMN artist_name;
ALTER TABLE listen_history_item DROP COLUMN artist_id;
ALTER TABLE listen_history_item DROP COLUMN title;
ALTER TABLE listen_history_item DROP COLUMN device;
ALTER TABLE listen_history_item DROP COLUMN source;
ALTER TABLE listen_history_item DROP COLUMN zone_name;
ALTER TABLE listen_history_item DROP COLUMN zone_id;

CREATE TABLE user_tag_item_previous (
  tag_id INTEGER NOT NULL,
  entity_type TEXT NOT NULL CHECK (entity_type IN ('track', 'album', 'artist')),
  entity_id TEXT NOT NULL,
  added_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (tag_id, entity_type, entity_id),
  CONSTRAINT user_tag_item_tag_fkey FOREIGN KEY (tag_id) REFERENCES user_tag(id) ON DELETE CASCADE
) WITHOUT ROWID;
INSERT INTO user_tag_item_previous(tag_id, entity_type, entity_id, added_at)
SELECT tag_id, entity_type, entity_id, added_at FROM user_tag_item
WHERE entity_type IN ('track', 'album', 'artist');
DROP TABLE user_tag_item;
ALTER TABLE user_tag_item_previous RENAME TO user_tag_item;
CREATE INDEX idx_user_tag_item_entity ON user_tag_item(entity_type, entity_id, tag_id);
CREATE TRIGGER user_data_changed_user_tag_item_insert AFTER INSERT ON user_tag_item BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;
CREATE TRIGGER user_data_changed_user_tag_item_delete AFTER DELETE ON user_tag_item BEGIN
  UPDATE user_data_change_state SET revision = revision + 1, changed_at = CURRENT_TIMESTAMP WHERE singleton = 1;
END;

DELETE FROM user_tag WHERE query_json IS NOT NULL;
ALTER TABLE user_tag DROP COLUMN description;
ALTER TABLE user_tag DROP COLUMN color;
ALTER TABLE user_tag DROP COLUMN query_json;

DROP TRIGGER IF EXISTS user_data_changed_listen_later_delete;
DROP TRIGGER IF EXISTS user_data_changed_listen_later_insert;
DROP TABLE IF EXISTS listen_later_setting;
DROP INDEX IF EXISTS idx_listen_later_user_added;
DROP TABLE IF EXISTS listen_later_item;
