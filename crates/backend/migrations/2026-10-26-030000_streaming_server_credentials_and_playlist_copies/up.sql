-- Server-wide TIDAL client and Qobuz app credentials, configured once by the
-- server owner so listeners only approve a TIDAL sign-in or enter their Qobuz
-- email and password. Sealed with AES-256-GCM under Secrets/streaming.key,
-- like every per-user credential; nothing here is plaintext.
CREATE TABLE provider_server_credential (
  provider TEXT NOT NULL PRIMARY KEY CHECK(provider IN ('tidal', 'qobuz')),
  sealed_credentials BLOB NOT NULL,
  updated_by INTEGER,
  updated_at BIGINT NOT NULL,
  CONSTRAINT provider_server_credential_user_fkey
    FOREIGN KEY (updated_by) REFERENCES user(id) ON DELETE SET NULL
) WITHOUT ROWID;

-- Read-only copies of TIDAL and Qobuz playlists a listener saved into
-- Parson. A copy keeps its tracks even if the service playlist changes or
-- disappears; saving again refreshes it.
CREATE TABLE provider_playlist_copy (
  user_id INTEGER NOT NULL,
  provider TEXT NOT NULL CHECK(provider IN ('tidal', 'qobuz')),
  provider_playlist_id TEXT NOT NULL,
  title TEXT NOT NULL,
  owner TEXT,
  cover_url TEXT,
  track_count INTEGER NOT NULL DEFAULT 0,
  duration REAL NOT NULL DEFAULT 0,
  payload TEXT NOT NULL,
  saved_at BIGINT NOT NULL,
  PRIMARY KEY (user_id, provider, provider_playlist_id),
  CONSTRAINT provider_playlist_copy_user_fkey
    FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
) WITHOUT ROWID;

CREATE INDEX provider_playlist_copy_saved_idx
  ON provider_playlist_copy(user_id, saved_at);
