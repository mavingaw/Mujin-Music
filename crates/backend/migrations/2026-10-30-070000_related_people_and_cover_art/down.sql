DELETE FROM metadata_task WHERE provider IN ('wikidata-query', 'coverart');
DROP TABLE IF EXISTS album_cover_art;
DROP TABLE IF EXISTS person_influence;
