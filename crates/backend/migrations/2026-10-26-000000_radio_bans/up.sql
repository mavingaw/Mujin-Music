-- Radio bans: artists, albums and tracks a listener never wants Radio,
-- Shuffle or recommendations to pick. Explicit plays still work.
CREATE TABLE user_radio_ban(
  user_id INTEGER NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('song', 'album', 'artist')),
  entity_id TEXT NOT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY(user_id, kind, entity_id),
  CONSTRAINT user_radio_ban_user_fkey FOREIGN KEY(user_id) REFERENCES user(id) ON DELETE CASCADE
);
