-- A composer's whole catalogue, not only the works on matched releases:
-- every work MusicBrainz lists for the composer (browse works by artist),
-- with the notable works Wikidata names (P800) and the facts Wikidata adds
-- (inception year, key, form). The works themselves live in `work`.
CREATE TABLE IF NOT EXISTS composer_catalogue(
  composer_mbid TEXT NOT NULL PRIMARY KEY,
  composer_name TEXT NOT NULL DEFAULT '',
  wikidata_id TEXT,
  -- What MusicBrainz reports (movements included) and what was stored.
  musicbrainz_total INTEGER NOT NULL DEFAULT 0,
  stored_works INTEGER NOT NULL DEFAULT 0,
  -- 1 when every page was read; 0 when the catalogue was capped.
  complete INTEGER NOT NULL DEFAULT 0,
  musicbrainz_fetched_at DATETIME,
  wikidata_fetched_at DATETIME,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS composer_catalogue_work(
  composer_mbid TEXT NOT NULL,
  work_mbid TEXT NOT NULL,
  wikidata_id TEXT,
  -- Position in the composer's Wikidata "notable work" list, or NULL.
  notable_rank INTEGER,
  PRIMARY KEY(composer_mbid, work_mbid)
);
CREATE INDEX IF NOT EXISTS idx_composer_catalogue_work_work ON composer_catalogue_work(work_mbid);

-- Text from the editorial sources after the first: the same article in
-- other Wikipedia languages, so an album or artist page can switch source.
-- The first source's text stays in editorial_text.
CREATE TABLE IF NOT EXISTS editorial_text_source(
  entity_type TEXT NOT NULL CHECK(entity_type IN ('artist', 'album', 'work')),
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
  PRIMARY KEY(entity_type, entity_id, kind, source, language)
);

-- Composers of works already in the library and matched items with a
-- Wikidata item are queued by the worker's seeding at startup.
