-- Profiles: several people share one Parson server. Every profile is a
-- Parson account, so history, favourites, ratings, radio taste and
-- recommendations stay separate without a second identity model. A
-- "profile" (as opposed to an "account") has no usable password: it is
-- created by someone in the household and entered by quick switching from a
-- device that is already signed in. A PIN (argon2) can guard any profile.
ALTER TABLE user ADD COLUMN profile_kind TEXT NOT NULL DEFAULT 'account'
  CHECK(profile_kind IN ('account', 'profile'));
ALTER TABLE user ADD COLUMN profile_color TEXT;
ALTER TABLE user ADD COLUMN profile_pin TEXT;
ALTER TABLE user ADD COLUMN quick_switch INTEGER NOT NULL DEFAULT 1;
-- Who created a managed profile (no foreign key so the column can be
-- dropped again; account deletion clears it).
ALTER TABLE user ADD COLUMN profile_owner_id INTEGER;

-- Last.fm API credentials for the whole server, supplied by its owner
-- (Last.fm requires an API account per application). Sealed with
-- AES-256-GCM under Secrets/streaming.key like every other credential.
CREATE TABLE scrobble_service_config (
  service TEXT NOT NULL PRIMARY KEY CHECK(service IN ('lastfm')),
  sealed_config BLOB NOT NULL,
  updated_by INTEGER,
  updated_at BIGINT NOT NULL,
  CONSTRAINT scrobble_service_config_user_fkey
    FOREIGN KEY (updated_by) REFERENCES user(id) ON DELETE SET NULL
) WITHOUT ROWID;

-- A profile's connection to Last.fm or ListenBrainz. The Last.fm session key
-- or ListenBrainz user token is sealed; nothing here is plaintext.
CREATE TABLE scrobble_connection (
  user_id INTEGER NOT NULL,
  service TEXT NOT NULL CHECK(service IN ('lastfm', 'listenbrainz')),
  account_name TEXT NOT NULL,
  sealed_secret BLOB NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  send_now_playing INTEGER NOT NULL DEFAULT 1,
  scrobble_radio INTEGER NOT NULL DEFAULT 1,
  status TEXT NOT NULL DEFAULT 'ok' CHECK(status IN ('ok', 'reauth_required')),
  last_error TEXT,
  last_scrobbled_at BIGINT,
  connected_at BIGINT NOT NULL,
  PRIMARY KEY (user_id, service),
  CONSTRAINT scrobble_connection_user_fkey
    FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
) WITHOUT ROWID;

-- Listens waiting for (or already handed to) a service. Rows survive
-- restarts and outages: the queue retries with backoff until the service
-- accepts or rejects each listen. Sent rows are kept briefly as a log.
CREATE TABLE scrobble_queue (
  id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER NOT NULL,
  service TEXT NOT NULL CHECK(service IN ('lastfm', 'listenbrainz')),
  dedupe_key TEXT NOT NULL,
  song_id TEXT,
  artist TEXT NOT NULL,
  track TEXT NOT NULL,
  album TEXT,
  album_artist TEXT,
  duration_seconds INTEGER,
  track_number INTEGER,
  release_mbid TEXT,
  isrc TEXT,
  listened_at BIGINT NOT NULL,
  chosen_by_user INTEGER NOT NULL DEFAULT 1,
  status TEXT NOT NULL DEFAULT 'pending'
    CHECK(status IN ('pending', 'sent', 'ignored', 'failed')),
  attempts INTEGER NOT NULL DEFAULT 0,
  next_attempt_at BIGINT NOT NULL,
  last_error TEXT,
  created_at BIGINT NOT NULL,
  sent_at BIGINT,
  CONSTRAINT scrobble_queue_unique UNIQUE (user_id, service, dedupe_key),
  CONSTRAINT scrobble_queue_user_fkey
    FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
);

CREATE INDEX scrobble_queue_due_idx ON scrobble_queue(status, next_attempt_at);
CREATE INDEX scrobble_queue_user_idx ON scrobble_queue(user_id, service, id DESC);

-- Progress of a listening-history import from a service into Parson.
CREATE TABLE scrobble_history_import (
  user_id INTEGER NOT NULL,
  service TEXT NOT NULL CHECK(service IN ('lastfm', 'listenbrainz')),
  status TEXT NOT NULL CHECK(status IN ('running', 'done', 'failed')),
  fetched INTEGER NOT NULL DEFAULT 0,
  matched INTEGER NOT NULL DEFAULT 0,
  imported INTEGER NOT NULL DEFAULT 0,
  error TEXT,
  started_at BIGINT NOT NULL,
  finished_at BIGINT,
  PRIMARY KEY (user_id, service),
  CONSTRAINT scrobble_history_import_user_fkey
    FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
) WITHOUT ROWID;
