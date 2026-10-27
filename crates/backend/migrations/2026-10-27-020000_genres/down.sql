DELETE FROM editorial_text WHERE entity_type = 'genre';
DELETE FROM editorial_profile WHERE entity_type = 'genre';
DELETE FROM metadata_task WHERE entity_type = 'genre';
DROP TABLE album_genre_edit;
DROP TABLE genre_link;
