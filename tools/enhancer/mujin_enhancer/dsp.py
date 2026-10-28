"""Chunked overlap-add processing and resampling, in plain NumPy.

Models see bounded chunks: each chunk is processed with `pad` extra samples
of real audio on both sides (discarded afterwards, because a model's output
is unreliable near the edges of its input) and neighbouring chunks are
joined with a linear crossfade of `overlap` samples, normalised so that the
weights always sum to one.
"""

from __future__ import annotations

from collections.abc import Callable
from math import gcd

import numpy as np

ModelFn = Callable[[np.ndarray], np.ndarray]


def chunk_starts(total: int, chunk: int, overlap: int) -> list[int]:
    """Chunk starts covering `total` samples, without a redundant last chunk."""
    if chunk <= 0 or overlap < 0 or overlap * 2 > chunk:
        raise ValueError("chunk must be positive and overlap at most half a chunk")
    hop = chunk - overlap
    starts = [0]
    while starts[-1] + chunk < total:
        starts.append(starts[-1] + hop)
    return starts


def crossfade_weights(length: int, overlap: int, fade_in: bool, fade_out: bool) -> np.ndarray:
    weights = np.ones(length, dtype=np.float64)
    fade = min(overlap, length)
    if fade:
        ramp = (np.arange(fade, dtype=np.float64) + 0.5) / fade
        if fade_in:
            weights[:fade] = ramp
        if fade_out:
            weights[-fade:] = np.minimum(weights[-fade:], ramp[::-1])
    return weights


def process_in_chunks(
    audio: np.ndarray,
    model: ModelFn,
    chunk: int,
    overlap: int,
    pad: int = 0,
    progress: Callable[[float], None] | None = None,
) -> np.ndarray:
    """Runs `model` over `audio` (`[channels, samples]`) chunk by chunk."""
    if audio.ndim != 2:
        raise ValueError("audio must be [channels, samples]")
    channels, total = audio.shape
    if total == 0:
        return audio.copy()
    if total <= chunk:
        out = np.asarray(model(audio), dtype=np.float64)
        _check(out, audio.shape)
        if progress:
            progress(1.0)
        return out
    starts = chunk_starts(total, chunk, overlap)
    out = np.zeros((channels, total), dtype=np.float64)
    weight_sum = np.zeros(total, dtype=np.float64)
    padded_length = chunk + 2 * pad
    for index, start in enumerate(starts):
        end = min(start + chunk, total)
        valid = end - start
        # Real audio pads each side; at the file's own edges the window
        # shifts inwards instead of padding with silence.
        padded_start = max(0, start - pad)
        if pad and end == total:
            padded_start = max(0, total - padded_length)
        window = audio[:, padded_start : padded_start + padded_length]
        if window.shape[1] < padded_length:
            window = np.pad(window, ((0, 0), (0, padded_length - window.shape[1])))
        result = np.asarray(model(window), dtype=np.float64)
        _check(result, window.shape)
        offset = start - padded_start
        piece = result[:, offset : offset + valid]
        weights = crossfade_weights(
            valid, overlap, fade_in=index > 0, fade_out=index < len(starts) - 1
        )
        out[:, start:end] += piece * weights
        weight_sum[start:end] += weights
        if progress:
            progress((index + 1) / len(starts))
    if np.any(weight_sum <= 0):
        raise RuntimeError("overlap-add left samples uncovered")
    return out / weight_sum


def _check(result: np.ndarray, shape: tuple[int, ...]) -> None:
    if result.shape != shape:
        raise RuntimeError(f"the model returned {result.shape} for input {shape}")


def resample(audio: np.ndarray, from_rate: int, to_rate: int, half_width: int = 24) -> np.ndarray:
    """Band-limited resampling (Kaiser-windowed sinc), `[channels, samples]`.

    When reducing the rate the filter cut-off moves down with it, so nothing
    above the new Nyquist frequency folds back.
    """
    if from_rate == to_rate:
        return audio.astype(np.float64, copy=True)
    if from_rate <= 0 or to_rate <= 0:
        raise ValueError("sample rates must be positive")
    divisor = gcd(from_rate, to_rate)
    up, down = to_rate // divisor, from_rate // divisor
    channels, total = audio.shape
    out_total = int(round(total * up / down))
    cutoff = min(1.0, to_rate / from_rate) * 0.97
    width = int(np.ceil(half_width / cutoff))
    beta = 8.6
    out = np.empty((channels, out_total), dtype=np.float64)
    block = 8192
    padded = np.pad(audio.astype(np.float64), ((0, 0), (width, width + 1)))
    taps = np.arange(-width + 1, width + 1)
    for begin in range(0, out_total, block):
        n = np.arange(begin, min(out_total, begin + block))
        position = n * down / up
        base = np.floor(position).astype(np.int64)
        frac = position - base
        index = base[:, None] + taps[None, :]
        distance = taps[None, :] - frac[:, None]
        kernel = cutoff * np.sinc(cutoff * distance)
        window = np.i0(beta * np.sqrt(np.clip(1 - (distance / (width + 1)) ** 2, 0, 1))) / np.i0(
            beta
        )
        kernel *= window
        for channel in range(channels):
            out[channel, begin : begin + len(n)] = np.sum(
                padded[channel, index + width] * kernel, axis=1
            )
    return out
