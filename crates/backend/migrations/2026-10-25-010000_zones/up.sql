-- Per-zone settings shared by every client: display name overrides and
-- volume safety limits. Zone ids are `sonos:<RINCON_...>`, `client:<device>`
-- or `linked:<uuid>`.
CREATE TABLE zone_setting(
  zone_id TEXT NOT NULL PRIMARY KEY,
  name TEXT,
  max_volume INTEGER,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT zone_setting_volume_check CHECK(max_volume IS NULL OR (max_volume >= 1 AND max_volume <= 100))
);
