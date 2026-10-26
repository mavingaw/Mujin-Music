-- The match check behind every MusicBrainz release applied to an album:
-- how the release was chosen (release id in the tags, chosen by hand, or
-- found automatically), whether it passed the check, and the evidence
-- (track count, track lengths, year, label, conductor and ensemble) as
-- JSON. An automatic match that fails keeps only this row: the album
-- keeps its file-tag data and asks a person to identify it.
CREATE TABLE IF NOT EXISTS album_release_match(
  album_id TEXT NOT NULL PRIMARY KEY,
  status TEXT NOT NULL CHECK(status IN ('verified', 'unverified')),
  method TEXT NOT NULL CHECK(method IN ('automatic', 'tagged', 'identified')),
  release_mbid TEXT NOT NULL,
  release_title TEXT NOT NULL,
  release_artist TEXT NOT NULL,
  score INTEGER NOT NULL,
  threshold INTEGER NOT NULL,
  details_json TEXT NOT NULL,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Albums matched automatically before the check existed are looked up
-- again, so a wrong release they took on is caught.
UPDATE metadata_task
SET status = 'pending', attempts = 0, last_error = NULL, not_before = NULL,
    updated_at = CURRENT_TIMESTAMP
WHERE provider = 'musicbrainz' AND entity_type = 'album' AND status IN ('done', 'skipped');
