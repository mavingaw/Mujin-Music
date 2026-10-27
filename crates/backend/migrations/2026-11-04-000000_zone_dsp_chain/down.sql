-- Back to the five-knob block: the first low shelf is the bass, the first
-- high shelf the treble, the headroom the preamp. Anything else the chain
-- held (other bands, convolution, a DSP rate) has no place in it.
UPDATE zone_setting
SET dsp = json_object(
    'enabled', json(CASE WHEN json_extract(dsp, '$.enabled') THEN 'true' ELSE 'false' END),
    'preamp_db', CASE WHEN json_extract(dsp, '$.headroom.enabled')
        THEN max(-24, min(0, cast(round(json_extract(dsp, '$.headroom.gainDb')) AS INTEGER)))
        ELSE 0 END,
    'bass_db', CASE WHEN json_extract(dsp, '$.eq.enabled') THEN coalesce((
        SELECT max(-12, min(12, cast(round(json_extract(band.value, '$.gainDb')) AS INTEGER)))
        FROM json_each(dsp, '$.eq.bands') AS band
        WHERE json_extract(band.value, '$.type') = 'lowshelf'
        ORDER BY band.key LIMIT 1), 0) ELSE 0 END,
    'treble_db', CASE WHEN json_extract(dsp, '$.eq.enabled') THEN coalesce((
        SELECT max(-12, min(12, cast(round(json_extract(band.value, '$.gainDb')) AS INTEGER)))
        FROM json_each(dsp, '$.eq.bands') AS band
        WHERE json_extract(band.value, '$.type') = 'highshelf'
        ORDER BY band.key LIMIT 1), 0) ELSE 0 END,
    'crossfeed', json(CASE WHEN json_extract(dsp, '$.crossfeed.enabled') THEN 'true' ELSE 'false' END)
)
WHERE dsp IS NOT NULL
  AND json_valid(dsp)
  AND json_type(dsp, '$.headroom') IS NOT NULL;
