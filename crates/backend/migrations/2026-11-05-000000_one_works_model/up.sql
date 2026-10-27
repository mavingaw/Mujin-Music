-- One works model. `work` held what MusicBrainz says about works (from
-- album matching and composer catalogues) while `work_entity` held the
-- library's works grouped from tags, with a second MusicBrainz cache
-- (`musicbrainz_work`, `musicbrainz_composer`) and a second track link
-- table. Now `work` holds every work: `mbid` is the MusicBrainz work id,
-- or a `wk_…` id for a work known only from its tags, and the library
-- columns say which works the library performs. `track_work` holds every
-- track-to-work link: `library` links are rebuilt from tags first, then
-- MusicBrainz; `musicbrainz` links come from album matching.

ALTER TABLE work ADD COLUMN instrumentation TEXT;
ALTER TABLE work ADD COLUMN scoring TEXT;
ALTER TABLE work ADD COLUMN composed_year INTEGER;
ALTER TABLE work ADD COLUMN period TEXT;
ALTER TABLE work ADD COLUMN tagged INTEGER NOT NULL DEFAULT 0;
ALTER TABLE work ADD COLUMN in_library INTEGER NOT NULL DEFAULT 0;
ALTER TABLE work ADD COLUMN created_at DATETIME;
-- What MusicBrainz said about the work is stored (not only what the
-- library's tags say).
ALTER TABLE work ADD COLUMN musicbrainz_facts INTEGER NOT NULL DEFAULT 0;
UPDATE work SET created_at = updated_at, musicbrainz_facts = 1;

-- What the separate MusicBrainz work lookups found.
INSERT INTO work (mbid, title, form, composed, composer_mbid, composer_name,
                  composer_normalized, parent_mbid, created_at, musicbrainz_facts)
SELECT mw.mbid, mw.title, mw.work_type, CAST(mw.composed_year AS TEXT),
       json_extract(mw.composer_mbids_json, '$[0]'), mc.name, mc.normalized_name,
       mw.parent_mbid, mw.fetched_at, 1
FROM musicbrainz_work mw
LEFT JOIN musicbrainz_composer mc
  ON mc.mbid = json_extract(mw.composer_mbids_json, '$[0]') AND mc.status = 'ok'
WHERE mw.status = 'ok' AND mw.title IS NOT NULL
ON CONFLICT(mbid) DO UPDATE SET
  form = COALESCE(work.form, excluded.form),
  composed = COALESCE(work.composed, excluded.composed),
  composer_mbid = COALESCE(work.composer_mbid, excluded.composer_mbid),
  composer_name = COALESCE(work.composer_name, excluded.composer_name),
  composer_normalized = COALESCE(work.composer_normalized, excluded.composer_normalized),
  parent_mbid = COALESCE(work.parent_mbid, excluded.parent_mbid),
  musicbrainz_facts = 1;

-- The library's works, under their MusicBrainz id when they have one.
INSERT INTO work (mbid, title, form, instrumentation, scoring, composed_year, period,
                  tagged, in_library, created_at)
SELECT COALESCE(we.musicbrainz_work_id, we.id), we.title, we.form, we.instrumentation,
       we.scoring, we.composed_year, we.period, we.tagged, 1, we.created_at
FROM work_entity we WHERE true
ON CONFLICT(mbid) DO UPDATE SET
  form = COALESCE(excluded.form, work.form),
  instrumentation = COALESCE(excluded.instrumentation, work.instrumentation),
  scoring = COALESCE(excluded.scoring, work.scoring),
  composed_year = COALESCE(excluded.composed_year, work.composed_year),
  period = COALESCE(excluded.period, work.period),
  tagged = MAX(work.tagged, excluded.tagged),
  in_library = 1,
  created_at = MIN(COALESCE(work.created_at, excluded.created_at), excluded.created_at);
CREATE INDEX idx_work_library_form ON work(in_library, form);
CREATE INDEX idx_work_library_period ON work(in_library, period);

-- Every track-to-work link in one table.
ALTER TABLE track_work RENAME TO track_work_previous;
CREATE TABLE track_work(
  track_id TEXT NOT NULL,
  album_id TEXT NOT NULL DEFAULT '',
  work_mbid TEXT NOT NULL,
  recording_mbid TEXT,
  movement TEXT,
  movement_number INTEGER,
  -- `library`: rebuilt from tags, then MusicBrainz; `musicbrainz`: album
  -- matching (kept until the album is matched again).
  source TEXT NOT NULL,
  PRIMARY KEY(track_id, work_mbid, source)
);
CREATE INDEX idx_track_work_link_work ON track_work(work_mbid, source, track_id);
CREATE INDEX idx_track_work_link_album ON track_work(album_id, source);
INSERT INTO track_work (track_id, album_id, work_mbid, recording_mbid, source)
SELECT track_id, album_id, work_mbid, recording_mbid, source FROM editorial_track_work
WHERE true ON CONFLICT DO NOTHING;
INSERT INTO track_work (track_id, album_id, work_mbid, movement, movement_number, source)
SELECT tw.track_id, COALESCE(t.album_id, ''), COALESCE(we.musicbrainz_work_id, tw.work_id),
       tw.movement, tw.movement_number, 'library'
FROM track_work_previous tw
LEFT JOIN work_entity we ON we.id = tw.work_id
LEFT JOIN track_entity t ON t.id = tw.track_id
WHERE true ON CONFLICT DO NOTHING;

ALTER TABLE work_composer RENAME TO work_composer_previous;
CREATE TABLE work_composer (
  work_id TEXT NOT NULL,
  artist_id TEXT NOT NULL,
  position INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (work_id, artist_id)
);
CREATE INDEX idx_work_composer_artist_work ON work_composer(artist_id, work_id);
INSERT INTO work_composer (work_id, artist_id, position)
SELECT COALESCE(we.musicbrainz_work_id, wc.work_id), wc.artist_id, wc.position
FROM work_composer_previous wc LEFT JOIN work_entity we ON we.id = wc.work_id
WHERE true ON CONFLICT DO NOTHING;

-- Composer life dates the separate lookups found stay as composer facts;
-- from now on they come from the artist's MusicBrainz profile.
ALTER TABLE composer_entity ADD COLUMN country TEXT;
INSERT INTO composer_entity (artist_id, birth_year, death_year, country, source)
SELECT ar.id, mc.birth_year, mc.death_year, mc.country, 'musicbrainz'
FROM musicbrainz_composer mc
JOIN artist_entity ar ON ar.normalized_name = mc.normalized_name
WHERE mc.status = 'ok' AND mc.birth_year IS NOT NULL
  AND ar.id IN (SELECT artist_id FROM track_artist WHERE role = 'composer')
ON CONFLICT(artist_id) DO UPDATE SET
  country = excluded.country,
  birth_year = COALESCE(composer_entity.birth_year, excluded.birth_year),
  death_year = COALESCE(composer_entity.death_year, excluded.death_year),
  source = 'musicbrainz';

DROP TABLE work_composer_previous;
DROP TABLE track_work_previous;
DROP TABLE editorial_track_work;
DROP TABLE work_entity;
DROP TABLE musicbrainz_work;
DROP TABLE musicbrainz_composer;

-- The library's view of the one model, for the Compositions browser.
CREATE VIEW library_work AS
SELECT mbid AS id, title,
       CASE WHEN length(mbid) = 36 THEN mbid END AS musicbrainz_work_id,
       form, instrumentation, scoring, composed_year, period, tagged, created_at
FROM work WHERE in_library = 1;
CREATE VIEW library_track_work AS
SELECT track_id, work_mbid AS work_id, movement, movement_number
FROM track_work WHERE source = 'library';

-- Rebuild the library's works with the new rules.
DELETE FROM work_catalog_state;
