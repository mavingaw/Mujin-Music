-- ReplayGain loudness tags read by tag parser version 18, stored as JSON:
-- {"track_gain_db":-6.4,"track_peak":0.98,"album_gain_db":-7.1,"album_peak":1.0}.
ALTER TABLE raw_file_metadata ADD COLUMN loudness_json TEXT;
