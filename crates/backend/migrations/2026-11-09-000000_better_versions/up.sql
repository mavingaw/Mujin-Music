-- Better versions: lossless copies requested from FETCH on the home network,
-- the fake-lossless check, and AI-enhanced copies made by the enhancer
-- worker. Nothing here ever points at a file Mujin Music writes over: the
-- originals are only read, and new files are always written beside them.

-- Server-wide secrets for these integrations (the FETCH agent token,
-- Cloudflare Access service token, the enhancer's shared secret), sealed
-- with the same key and AEAD as the streaming credentials.
CREATE TABLE upgrade_secret(
  name TEXT NOT NULL PRIMARY KEY,
  sealed BLOB NOT NULL,
  updated_by INTEGER,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Spectral cutoff check per file version: a FLAC/ALAC/WAV whose spectrum
-- stops like an MP3's is a transcode ("fake lossless").
CREATE TABLE upgrade_spectral_check(
  path TEXT NOT NULL PRIMARY KEY,
  file_version TEXT NOT NULL,
  verdict TEXT NOT NULL CHECK(verdict IN ('genuine', 'transcode', 'inconclusive')),
  cutoff_hz INTEGER,
  confidence REAL NOT NULL DEFAULT 0,
  detail TEXT NOT NULL DEFAULT '',
  analyzed_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- One request to FETCH for a better copy of one album.
CREATE TABLE upgrade_request(
  id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
  album_id TEXT NOT NULL,
  album_title TEXT NOT NULL DEFAULT '',
  artist_name TEXT NOT NULL DEFAULT '',
  query TEXT NOT NULL,
  reason TEXT NOT NULL DEFAULT '',
  origin TEXT NOT NULL DEFAULT 'manual' CHECK(origin IN ('manual', 'automatic')),
  requested_by INTEGER,
  state TEXT NOT NULL,
  message TEXT NOT NULL DEFAULT '',
  next_poll_at INTEGER NOT NULL DEFAULT 0,
  poll_failures INTEGER NOT NULL DEFAULT 0,
  quiet_polls INTEGER NOT NULL DEFAULT 0,
  result_album_id TEXT,
  result_folder TEXT,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  finished_at DATETIME
);
CREATE INDEX idx_upgrade_request_album ON upgrade_request(album_id, created_at);
CREATE INDEX idx_upgrade_request_state ON upgrade_request(state, next_poll_at);

-- FETCH jobs created by a request, with their last known status.
CREATE TABLE upgrade_request_job(
  request_id INTEGER NOT NULL,
  job_id TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'queued',
  progress REAL,
  speed TEXT NOT NULL DEFAULT '',
  eta TEXT NOT NULL DEFAULT '',
  title TEXT NOT NULL DEFAULT '',
  engine TEXT NOT NULL DEFAULT '',
  dest TEXT NOT NULL DEFAULT '',
  error TEXT NOT NULL DEFAULT '',
  note TEXT NOT NULL DEFAULT '',
  size TEXT NOT NULL DEFAULT '',
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY(request_id, job_id),
  FOREIGN KEY(request_id) REFERENCES upgrade_request(id) ON DELETE CASCADE
);

-- Files written by the enhancer: always new files, beside the original.
CREATE TABLE upgrade_enhanced_file(
  path TEXT NOT NULL PRIMARY KEY,
  source_path TEXT NOT NULL,
  model TEXT NOT NULL,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_upgrade_enhanced_source ON upgrade_enhanced_file(source_path);

-- The offline enhance queue: one row per track, resumable after a restart.
CREATE TABLE upgrade_enhance_job(
  id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
  song_id TEXT NOT NULL,
  album_id TEXT NOT NULL DEFAULT '',
  source_path TEXT NOT NULL,
  title TEXT NOT NULL DEFAULT '',
  state TEXT NOT NULL DEFAULT 'queued'
    CHECK(state IN ('queued', 'running', 'done', 'failed', 'cancelled', 'skipped')),
  worker_job_id TEXT,
  progress REAL NOT NULL DEFAULT 0,
  output_path TEXT,
  message TEXT NOT NULL DEFAULT '',
  batch TEXT NOT NULL DEFAULT '',
  attempts INTEGER NOT NULL DEFAULT 0,
  created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_upgrade_enhance_job_state ON upgrade_enhance_job(state, id);
CREATE INDEX idx_upgrade_enhance_job_source ON upgrade_enhance_job(source_path);
