-- Genres: MusicBrainz genres for albums (release groups) and artists, and
-- album genres an administrator edited by hand. Tag genres stay in
-- album_genre/track_genre, written by the indexer. Like the other
-- editorial tables these hold catalog ids without foreign keys so a rescan
-- that rebuilds catalog rows keeps them.
CREATE TABLE genre_link(
  entity_type TEXT NOT NULL CHECK(entity_type IN ('artist', 'album')),
  entity_id TEXT NOT NULL,
  -- The name as the source gave it; it is normalised when read.
  name TEXT NOT NULL,
  source TEXT NOT NULL,
  -- MusicBrainz vote count.
  weight INTEGER NOT NULL DEFAULT 1,
  mbid TEXT,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY(entity_type, entity_id, name, source)
);

-- Genres chosen by hand replace tag and online genres for that album. An
-- empty list means "no genres".
CREATE TABLE album_genre_edit(
  album_id TEXT NOT NULL PRIMARY KEY,
  genres_json TEXT NOT NULL DEFAULT '[]',
  edited_by INTEGER,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Genre pages get their About text from Wikipedia through Wikidata, so the
-- editorial profile and text tables now also hold genres (keyed by slug).
CREATE TABLE editorial_profile_new(
  entity_type TEXT NOT NULL CHECK(entity_type IN ('artist', 'album', 'genre')),
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
  entity_type TEXT NOT NULL CHECK(entity_type IN ('artist', 'album', 'genre')),
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
