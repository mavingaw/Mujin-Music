-- Influences and followers from Wikidata (P737 "influenced by" and its
-- reverse), for the related-people sections of person pages. Keyed by the
-- enrichment entity id (a library artist id or `mb:<MusicBrainz id>`)
-- without foreign keys, like the other editorial tables.
CREATE TABLE IF NOT EXISTS person_influence(
  artist_id TEXT NOT NULL,
  direction TEXT NOT NULL CHECK(direction IN ('influence', 'follower')),
  wikidata_id TEXT NOT NULL,
  name TEXT NOT NULL,
  description TEXT,
  musicbrainz_id TEXT,
  -- Commons file name of the Wikidata image (P18).
  image TEXT,
  position INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(artist_id, direction, wikidata_id)
);

-- Front covers fetched from the Cover Art Archive for albums with no
-- artwork of their own. The file lives under the data directory's
-- "Album Covers" folder.
CREATE TABLE IF NOT EXISTS album_cover_art(
  album_id TEXT NOT NULL PRIMARY KEY,
  file_path TEXT NOT NULL,
  mime_type TEXT NOT NULL,
  source_url TEXT NOT NULL,
  release_mbid TEXT,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- People and albums matched before these stages existed are queued by the
-- worker's start-up seeding (enrichment::store::seed_tasks).
