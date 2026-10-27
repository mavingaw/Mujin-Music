-- Roon-style zone settings: icon, crossfade, volume leveling, a private
-- flag, and volume limits split into a soft comfort limit and a hard safety
-- range (the old single cap becomes the safety maximum).
ALTER TABLE zone_setting ADD COLUMN icon TEXT;
ALTER TABLE zone_setting ADD COLUMN crossfade_seconds INTEGER NOT NULL DEFAULT 0;
ALTER TABLE zone_setting ADD COLUMN leveling TEXT NOT NULL DEFAULT 'off';
ALTER TABLE zone_setting ADD COLUMN leveling_target_lufs INTEGER NOT NULL DEFAULT -18;
ALTER TABLE zone_setting ADD COLUMN leveling_adjustment_db INTEGER NOT NULL DEFAULT 0;
ALTER TABLE zone_setting ADD COLUMN comfort_volume INTEGER;
ALTER TABLE zone_setting ADD COLUMN safety_min_volume INTEGER;
ALTER TABLE zone_setting ADD COLUMN safety_max_volume INTEGER;
ALTER TABLE zone_setting ADD COLUMN private INTEGER NOT NULL DEFAULT 0;
UPDATE zone_setting SET safety_max_volume = max_volume WHERE max_volume IS NOT NULL;

-- EBU R128 loudness per track for volume leveling, measured once per file
-- version (fingerprint = size and modification time).
CREATE TABLE track_loudness(
  song_id TEXT NOT NULL PRIMARY KEY,
  fingerprint TEXT NOT NULL,
  integrated_lufs DOUBLE NOT NULL,
  true_peak_db DOUBLE NOT NULL,
  measured_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
