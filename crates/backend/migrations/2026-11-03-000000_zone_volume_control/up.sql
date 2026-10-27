-- Who carries out a zone's volume changes: Parson itself, or Home
-- Assistant (each request goes out as an event an automation acts on).
ALTER TABLE zone_setting ADD COLUMN volume_control TEXT NOT NULL DEFAULT 'parson';
