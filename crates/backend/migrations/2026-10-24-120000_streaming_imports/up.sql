-- Connections to external music services, used only to read a person's
-- playlists for import and (Spotify) to hand playback off to their own
-- devices. No audio is ever streamed from these services.
CREATE TABLE import_connection (
  user_id INTEGER NOT NULL,
  service TEXT NOT NULL,
  -- JSON: tokens, client id, developer token, API key. Never returned by the API.
  credentials TEXT NOT NULL DEFAULT '{}',
  account_label TEXT,
  expires_at BIGINT,
  connected_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (user_id, service),
  CONSTRAINT import_connection_user_fkey
    FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
);

-- Pending OAuth (PKCE) sign-ins; rows live for minutes.
CREATE TABLE import_auth_state (
  state TEXT PRIMARY KEY NOT NULL,
  user_id INTEGER NOT NULL,
  service TEXT NOT NULL,
  code_verifier TEXT NOT NULL,
  client_id TEXT NOT NULL,
  redirect_uri TEXT NOT NULL,
  created_at BIGINT NOT NULL,
  CONSTRAINT import_auth_state_user_fkey
    FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
);

CREATE TABLE import_job (
  id TEXT PRIMARY KEY NOT NULL,
  user_id INTEGER NOT NULL,
  service TEXT NOT NULL,
  source_kind TEXT NOT NULL,
  source_ref TEXT NOT NULL,
  name TEXT NOT NULL,
  status TEXT NOT NULL,
  error TEXT,
  source_url TEXT,
  playlist_id INTEGER,
  -- JSON array of imported items with their match results.
  items TEXT NOT NULL DEFAULT '[]',
  created_at BIGINT NOT NULL,
  updated_at BIGINT NOT NULL,
  CONSTRAINT import_job_user_fkey
    FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
);

CREATE INDEX import_job_user_updated_idx ON import_job(user_id, updated_at DESC);
