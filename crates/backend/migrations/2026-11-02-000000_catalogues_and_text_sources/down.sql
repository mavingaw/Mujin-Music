DELETE FROM metadata_task WHERE provider IN ('musicbrainz-works', 'wikipedia-sources');
DROP TABLE IF EXISTS editorial_text_source;
DROP INDEX IF EXISTS idx_composer_catalogue_work_work;
DROP TABLE IF EXISTS composer_catalogue_work;
DROP TABLE IF EXISTS composer_catalogue;
