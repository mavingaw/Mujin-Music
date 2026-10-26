-- Zones keep the same DSP chain as the rest of the server (headroom,
-- parametric EQ, crossfeed, convolution), not the old five-knob block.
-- The old block maps onto it exactly: its effective preamp (the preamp, or
-- the largest boost when that is lower) becomes the headroom, bass and
-- treble become 100 Hz and 10 kHz shelves (Q 0.707), and its crossfeed (a
-- 700 Hz feed at -4.5 dB) becomes the strong crossfeed.
UPDATE zone_setting
SET dsp = json_object(
    'enabled', json(CASE WHEN json_extract(dsp, '$.enabled') THEN 'true' ELSE 'false' END),
    'headroom', json_object(
        'enabled', json(CASE WHEN min(coalesce(json_extract(dsp, '$.preamp_db'), 0),
            -max(coalesce(json_extract(dsp, '$.bass_db'), 0),
                 coalesce(json_extract(dsp, '$.treble_db'), 0), 0)) < 0
            THEN 'true' ELSE 'false' END),
        'gainDb', CASE WHEN min(coalesce(json_extract(dsp, '$.preamp_db'), 0),
            -max(coalesce(json_extract(dsp, '$.bass_db'), 0),
                 coalesce(json_extract(dsp, '$.treble_db'), 0), 0)) < 0
            THEN min(coalesce(json_extract(dsp, '$.preamp_db'), 0),
                -max(coalesce(json_extract(dsp, '$.bass_db'), 0),
                     coalesce(json_extract(dsp, '$.treble_db'), 0), 0)) * 1.0
            ELSE -3.0 END
    ),
    'eq', json_object(
        'enabled', json(CASE WHEN coalesce(json_extract(dsp, '$.bass_db'), 0) != 0
            OR coalesce(json_extract(dsp, '$.treble_db'), 0) != 0 THEN 'true' ELSE 'false' END),
        'preset', 'custom',
        'bands', json_array(
            json_object('id', 'band-1', 'enabled', json('true'), 'type', 'lowshelf',
                'frequency', 100.0, 'gainDb', coalesce(json_extract(dsp, '$.bass_db'), 0) * 1.0,
                'q', 0.707),
            json_object('id', 'band-2', 'enabled', json('true'), 'type', 'highshelf',
                'frequency', 10000.0, 'gainDb', coalesce(json_extract(dsp, '$.treble_db'), 0) * 1.0,
                'q', 0.707)
        )
    ),
    'crossfeed', json_object(
        'enabled', json(CASE WHEN json_extract(dsp, '$.crossfeed') THEN 'true' ELSE 'false' END),
        'strength', 'strong'
    ),
    'convolution', json_object('enabled', json('false'), 'filterId', NULL),
    'sampleRate', json_object('enabled', json('false'), 'mode', 'device')
)
WHERE dsp IS NOT NULL
  AND json_valid(dsp)
  AND json_type(dsp, '$.headroom') IS NULL;
