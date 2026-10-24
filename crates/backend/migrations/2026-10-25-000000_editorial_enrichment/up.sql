-- Opt-in editorial enrichment (MusicBrainz, Wikidata, Wikipedia, Wikimedia
-- Commons) and per-user notes. Rows are keyed by the catalog's stable entity
-- ids without foreign keys so a rescan that rebuilds catalog rows keeps them.
CREATE TABLE server_setting(
  key TEXT NOT NULL PRIMARY KEY,
  value_json TEXT NOT NULL,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Identity and facts resolved for an artist or album.
CREATE TABLE editorial_profile(
  entity_type TEXT NOT NULL CHECK(entity_type IN ('artist', 'album')),
  entity_id TEXT NOT NULL,
  musicbrainz_id TEXT,
  release_group_mbid TEXT,
  wikidata_id TEXT,
  profile_json TEXT NOT NULL DEFAULT '{}',
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY(entity_type, entity_id)
);

-- Licensed text: an artist biography, album background or album reception.
CREATE TABLE editorial_text(
  entity_type TEXT NOT NULL CHECK(entity_type IN ('artist', 'album')),
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

-- A freely licensed image stored under the data directory.
CREATE TABLE editorial_image(
  entity_type TEXT NOT NULL CHECK(entity_type IN ('artist', 'album')),
  entity_id TEXT NOT NULL,
  file_name TEXT NOT NULL,
  mime_type TEXT NOT NULL,
  source_url TEXT NOT NULL,
  page_url TEXT NOT NULL,
  author TEXT NOT NULL,
  license TEXT NOT NULL,
  license_url TEXT,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY(entity_type, entity_id)
);

-- Credits from an external source. track_id is '' for release-level credits.
CREATE TABLE enriched_credit(
  album_id TEXT NOT NULL,
  track_id TEXT NOT NULL,
  name TEXT NOT NULL,
  normalized_name TEXT NOT NULL,
  role TEXT NOT NULL,
  instrument TEXT NOT NULL DEFAULT '',
  source TEXT NOT NULL,
  source_artist_id TEXT,
  PRIMARY KEY(album_id, track_id, normalized_name, role, instrument, source)
);
CREATE INDEX idx_enriched_credit_name ON enriched_credit(normalized_name);
CREATE INDEX idx_enriched_credit_source_artist ON enriched_credit(source_artist_id);

CREATE TABLE user_note(
  user_id INTEGER NOT NULL,
  entity_type TEXT NOT NULL CHECK(entity_type IN ('artist', 'album')),
  entity_id TEXT NOT NULL,
  body TEXT NOT NULL,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY(user_id, entity_type, entity_id),
  CONSTRAINT user_note_user_id_fkey FOREIGN KEY(user_id) REFERENCES "user"(id) ON DELETE CASCADE
);
