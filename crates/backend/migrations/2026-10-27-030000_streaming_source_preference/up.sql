-- Which copy plays when an album is in the local library and on TIDAL or
-- Qobuz: the best-sounding one, or always one source first.
CREATE TABLE provider_source_preference (
  user_id INTEGER NOT NULL PRIMARY KEY,
  source TEXT NOT NULL DEFAULT 'highest_quality'
    CHECK(source IN ('highest_quality', 'local', 'tidal', 'qobuz')),
  updated_at BIGINT NOT NULL,
  CONSTRAINT provider_source_preference_user_fkey
    FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
) WITHOUT ROWID;
