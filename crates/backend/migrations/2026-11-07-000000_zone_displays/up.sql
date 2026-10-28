-- Displays: TVs, tablets and spare browsers that show what a zone plays,
-- full screen and without controls. Each holds a key that can only read
-- the now-playing state of its zone (or of whichever zone is playing).
CREATE TABLE zone_display(
  id TEXT PRIMARY KEY NOT NULL,
  user_id INTEGER NOT NULL,
  name TEXT NOT NULL,
  -- NULL follows whichever of the listener's zones is playing.
  zone_id TEXT,
  lyrics TEXT NOT NULL DEFAULT 'auto' CHECK (lyrics IN ('auto', 'off')),
  key_hash TEXT NOT NULL UNIQUE,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  last_seen_at DATETIME,
  CONSTRAINT zone_display_user_fkey FOREIGN KEY(user_id) REFERENCES user(id) ON DELETE CASCADE
);
CREATE INDEX zone_display_user ON zone_display(user_id);
