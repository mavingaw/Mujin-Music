-- The names a MusicBrainz person is known by: their own name, the names
-- releases credit them as ("Jonathan Davis" for Q-Tip) and their aliases.
-- Used to merge a person credited under different names in tags and on
-- MusicBrainz into one identity. Credited names already stored with album
-- credits are read from enriched_credit directly, so nothing is copied.
CREATE TABLE IF NOT EXISTS person_alias(
  mbid TEXT NOT NULL,
  normalized_name TEXT NOT NULL,
  name TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('name', 'credit', 'alias')),
  PRIMARY KEY(mbid, normalized_name)
);
CREATE INDEX IF NOT EXISTS idx_person_alias_name ON person_alias(normalized_name);
