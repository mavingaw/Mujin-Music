-- Technical properties and tag credits read by tag parser version 17.
-- codec, container, sample_rate, channels and bitrate already exist on
-- raw_file_metadata; bitrate is stored in kbps.
ALTER TABLE raw_file_metadata ADD COLUMN bit_depth INTEGER;
ALTER TABLE raw_file_metadata ADD COLUMN credits_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE raw_file_metadata ADD COLUMN label TEXT;
ALTER TABLE raw_file_metadata ADD COLUMN isrc TEXT;
