-- Radio: remember what a generated queue was seeded from, and per-user
-- Radio preferences (auto-continue after the queue ends, coming-up notice).
ALTER TABLE playback_queue ADD COLUMN seed_kind TEXT;
ALTER TABLE playback_queue ADD COLUMN seed_id TEXT;

CREATE TABLE user_radio_settings(
  user_id INTEGER NOT NULL PRIMARY KEY,
  enabled INTEGER NOT NULL DEFAULT 1,
  coming_up_notice INTEGER NOT NULL DEFAULT 1,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT user_radio_settings_user_fkey FOREIGN KEY(user_id) REFERENCES user(id) ON DELETE CASCADE
);
