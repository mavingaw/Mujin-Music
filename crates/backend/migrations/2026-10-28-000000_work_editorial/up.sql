-- Works (compositions) get their own About text from Wikipedia, found
-- through the work's MusicBrainz Wikipedia or Wikidata link, so the
-- editorial profile and text tables now also hold works (keyed by MBID).
CREATE TABLE editorial_profile_new(
  entity_type TEXT NOT NULL CHECK(entity_type IN ('artist', 'album', 'genre', 'work')),
  entity_id TEXT NOT NULL,
  musicbrainz_id TEXT,
  release_group_mbid TEXT,
  wikidata_id TEXT,
  profile_json TEXT NOT NULL DEFAULT '{}',
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY(entity_type, entity_id)
);
INSERT INTO editorial_profile_new SELECT entity_type, entity_id, musicbrainz_id,
  release_group_mbid, wikidata_id, profile_json, updated_at FROM editorial_profile;
DROP TABLE editorial_profile;
ALTER TABLE editorial_profile_new RENAME TO editorial_profile;

CREATE TABLE editorial_text_new(
  entity_type TEXT NOT NULL CHECK(entity_type IN ('artist', 'album', 'genre', 'work')),
  entity_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('biography', 'background', 'reception')),
  source TEXT NOT NULL,
  language TEXT NOT NULL,
  title TEXT NOT NULL,
  body TEXT NOT NULL,
  url TEXT NOT NULL,
  license TEXT NOT NULL,
  license_url TEXT NOT NULL,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY(entity_type, entity_id, kind)
);
INSERT INTO editorial_text_new SELECT entity_type, entity_id, kind, source, language,
  title, body, url, license, license_url, updated_at FROM editorial_text;
DROP TABLE editorial_text;
ALTER TABLE editorial_text_new RENAME TO editorial_text;

-- Text no longer waits for Wikidata: everything MusicBrainz matched is
-- queued for the Wikipedia stage directly, and the Wikipedia queue now
-- finds a genre's Wikidata item instead of the Wikidata API.
INSERT INTO metadata_task (provider, entity_type, entity_id)
SELECT 'wikipedia', entity_type, entity_id FROM editorial_profile
WHERE entity_type IN ('artist', 'album') AND true
ON CONFLICT(provider, entity_type, entity_id) DO UPDATE SET
  status = 'pending', attempts = 0, last_error = NULL, not_before = NULL
  WHERE metadata_task.status <> 'done';
INSERT INTO metadata_task (provider, entity_type, entity_id)
SELECT 'wikipedia', 'genre', entity_id FROM metadata_task
WHERE provider = 'wikidata' AND entity_type = 'genre' AND status <> 'done'
ON CONFLICT(provider, entity_type, entity_id) DO UPDATE SET
  status = 'pending', attempts = 0, last_error = NULL, not_before = NULL;
DELETE FROM metadata_task
WHERE provider = 'wikidata' AND entity_type = 'genre' AND status <> 'done';
