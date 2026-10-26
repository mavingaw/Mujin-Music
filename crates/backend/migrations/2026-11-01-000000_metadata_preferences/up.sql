-- Where each album fact comes from, and the facts a person froze.
--
-- Release facts tagged on the files (catalog number, country, original
-- date, barcode) are read by tag parser version 19 into release_json.
ALTER TABLE raw_file_metadata ADD COLUMN release_json TEXT;

-- Per album and field: use the file tags or the matched online release,
-- instead of the library default. Fields: original_date, release_date,
-- label, catalog_number, country, barcode, cover, performers, credits.
CREATE TABLE IF NOT EXISTS album_field_preference(
  album_id TEXT NOT NULL,
  field TEXT NOT NULL,
  source TEXT NOT NULL CHECK(source IN ('tags', 'online')),
  updated_by INTEGER,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY(album_id, field)
);

-- A value a person typed. It wins over tags and every later lookup
-- (re-identify, "Look up online metadata again") until it is reverted.
CREATE TABLE IF NOT EXISTS album_field_edit(
  album_id TEXT NOT NULL,
  field TEXT NOT NULL,
  value_json TEXT NOT NULL,
  edited_by INTEGER,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY(album_id, field)
);

-- Albums whose credits a person edited. The edited list (name, role,
-- instrument, tracks) replaces tag and online credits for the album on
-- every page, until it is reverted.
CREATE TABLE IF NOT EXISTS album_credit_edit(
  album_id TEXT NOT NULL PRIMARY KEY,
  credits_json TEXT NOT NULL DEFAULT '[]',
  edited_by INTEGER,
  updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
