-- Streaming-service accounts (TIDAL, Qobuz) and the streaming albums and
-- tracks each listener adds to their library.
--
-- Credentials, tokens and pending device codes are sealed with AES-256-GCM
-- under Secrets/streaming.key before they reach this table; nothing here is
-- plaintext.
CREATE TABLE provider_account (
  user_id INTEGER NOT NULL,
  provider TEXT NOT NULL CHECK(provider IN ('tidal', 'qobuz')),
  sealed_credentials BLOB NOT NULL,
  sealed_session BLOB,
  sealed_pending BLOB,
  account_name TEXT,
  plan TEXT,
  quality TEXT NOT NULL CHECK(quality IN ('hi_res_max', 'hi_res_96', 'lossless', 'high', 'low')),
  status TEXT NOT NULL DEFAULT 'configured'
    CHECK(status IN ('configured', 'pending', 'signed_in', 'expired', 'error')),
  status_message TEXT,
  last_library_sync BIGINT,
  created_at BIGINT NOT NULL,
  updated_at BIGINT NOT NULL,
  PRIMARY KEY (user_id, provider),
  CONSTRAINT provider_account_user_fkey
    FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
) WITHOUT ROWID;

CREATE TABLE provider_library_item (
  user_id INTEGER NOT NULL,
  provider TEXT NOT NULL CHECK(provider IN ('tidal', 'qobuz')),
  kind TEXT NOT NULL CHECK(kind IN ('album', 'track')),
  provider_item_id TEXT NOT NULL,
  title TEXT NOT NULL,
  normalized_title TEXT NOT NULL,
  artist_name TEXT NOT NULL DEFAULT '',
  normalized_artist TEXT NOT NULL DEFAULT '',
  album_title TEXT,
  cover_url TEXT,
  release_date TEXT,
  duration REAL NOT NULL DEFAULT 0,
  track_count INTEGER NOT NULL DEFAULT 0,
  format_label TEXT,
  payload TEXT NOT NULL,
  origin TEXT NOT NULL CHECK(origin IN ('parson', 'sync')),
  added_at BIGINT NOT NULL,
  PRIMARY KEY (user_id, provider, kind, provider_item_id),
  CONSTRAINT provider_library_item_user_fkey
    FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
) WITHOUT ROWID;

CREATE INDEX provider_library_item_title_idx
  ON provider_library_item(user_id, kind, normalized_title);
CREATE INDEX provider_library_item_added_idx
  ON provider_library_item(user_id, kind, added_at);
