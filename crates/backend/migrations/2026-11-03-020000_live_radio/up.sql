-- Live internet radio: stations a listener saved to My Live Radio (from the
-- radio-browser.info directory or added by URL) and stations played lately.
-- Each row keeps the whole station as JSON so saved stations keep working
-- when the directory is unreachable.
CREATE TABLE live_radio_station(
  user_id INTEGER NOT NULL,
  station_id TEXT NOT NULL,
  source TEXT NOT NULL CHECK (source IN ('directory', 'custom')),
  station_json TEXT NOT NULL,
  position INTEGER NOT NULL DEFAULT 0,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY(user_id, station_id),
  CONSTRAINT live_radio_station_user_fkey FOREIGN KEY(user_id) REFERENCES user(id) ON DELETE CASCADE
);
CREATE INDEX live_radio_station_by_id ON live_radio_station(station_id);

CREATE TABLE live_radio_recent(
  user_id INTEGER NOT NULL,
  station_id TEXT NOT NULL,
  station_json TEXT NOT NULL,
  played_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  -- Increases with every play, so plays in the same millisecond still order.
  play_order INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(user_id, station_id),
  CONSTRAINT live_radio_recent_user_fkey FOREIGN KEY(user_id) REFERENCES user(id) ON DELETE CASCADE
);

-- What Parson heard when it last opened a station: the format read from the
-- audio itself, shared by every listener on this server, so quality badges
-- show what a station really sends, not only what its listing claims.
CREATE TABLE live_radio_verified(
  station_id TEXT NOT NULL PRIMARY KEY,
  format_json TEXT NOT NULL,
  verified_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
