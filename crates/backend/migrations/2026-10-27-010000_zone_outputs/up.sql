-- Per-zone DSP (preamp, bass, treble, crossfeed) and device setup for
-- server-driven zones (exclusive mode, volume mode, maximum format), as
-- JSON objects. NULL means the defaults.
ALTER TABLE zone_setting ADD COLUMN dsp TEXT;
ALTER TABLE zone_setting ADD COLUMN output TEXT;
