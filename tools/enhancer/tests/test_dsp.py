import numpy as np
import pytest

from mujin_enhancer.dsp import chunk_starts, crossfade_weights, process_in_chunks, resample


def noise(channels: int, samples: int, seed: int = 1) -> np.ndarray:
    return np.random.default_rng(seed).standard_normal((channels, samples)) * 0.2


def test_chunk_starts_cover_everything_without_a_redundant_chunk():
    assert chunk_starts(100, 40, 10) == [0, 30, 60]
    assert chunk_starts(40, 40, 10) == [0]
    with pytest.raises(ValueError):
        chunk_starts(100, 40, 30)


def test_crossfades_sum_to_one_across_a_seam():
    fade_out = crossfade_weights(40, 10, fade_in=False, fade_out=True)
    fade_in = crossfade_weights(40, 10, fade_in=True, fade_out=False)
    assert np.allclose(fade_out[-10:] + fade_in[:10], 1.0)


def test_an_identity_model_reconstructs_the_signal_exactly():
    audio = noise(2, 10_000)
    out = process_in_chunks(audio, lambda chunk: chunk, chunk=1_000, overlap=200, pad=150)
    assert out.shape == audio.shape
    assert np.allclose(out, audio)


def test_padding_hides_a_models_edge_errors():
    def edgy(chunk: np.ndarray) -> np.ndarray:
        # Wrong near both edges of whatever it is given.
        out = chunk.copy()
        out[:, :50] += 1.0
        out[:, -50:] += 1.0
        return out

    audio = noise(1, 20_000)
    padded = process_in_chunks(audio, edgy, chunk=2_000, overlap=200, pad=100)
    # Only the file's own first and last 50 samples can carry the error.
    assert np.allclose(padded[:, 100:-100], audio[:, 100:-100])
    unpadded = process_in_chunks(audio, edgy, chunk=2_000, overlap=200, pad=0)
    assert not np.allclose(unpadded[:, 100:-100], audio[:, 100:-100])


def test_a_model_that_changes_the_shape_is_refused():
    with pytest.raises(RuntimeError):
        process_in_chunks(noise(1, 5_000), lambda c: c[:, :-1], chunk=1_000, overlap=100)


def test_progress_reaches_one():
    seen = []
    process_in_chunks(noise(1, 5_000), lambda c: c, 1_000, 100, 0, seen.append)
    assert seen[-1] == pytest.approx(1.0)


@pytest.mark.parametrize(
    ("source", "target"), [(48_000, 44_100), (22_050, 44_100), (44_100, 48_000)]
)
def test_resampling_keeps_frequency_level_and_duration(source: int, target: int):
    seconds = 1.0
    t = np.arange(int(source * seconds)) / source
    tone = 0.5 * np.sin(2 * np.pi * 1_000 * t)[None, :]
    out = resample(tone, source, target)
    assert out.shape[1] == round(source * seconds * target / source)
    middle = out[0, target // 10 : -target // 10]
    spectrum = np.abs(np.fft.rfft(middle * np.hanning(len(middle))))
    peak_hz = np.argmax(spectrum) * target / len(middle)
    assert abs(peak_hz - 1_000) < 5
    assert np.max(np.abs(middle)) == pytest.approx(0.5, abs=0.02)


def test_downsampling_removes_what_the_new_rate_cannot_hold():
    source, target = 48_000, 22_050
    t = np.arange(source) / source
    # 15 kHz is above the new Nyquist (11.025 kHz): it must not fold back.
    tone = 0.5 * np.sin(2 * np.pi * 15_000 * t)[None, :]
    out = resample(tone, source, target)
    assert np.max(np.abs(out[0, 2_000:-2_000])) < 0.02
    assert np.allclose(resample(tone, source, source), tone)
