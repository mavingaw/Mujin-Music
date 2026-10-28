"""Decoding with ffmpeg, and writing the 24-bit FLAC with the source's tags."""

from __future__ import annotations

import shutil
import subprocess  # noqa: S404 - ffmpeg is called with fixed argument lists
import tempfile
from pathlib import Path

import numpy as np
import soundfile as sf

from .dsp import resample


class AudioError(RuntimeError):
    pass


def ffmpeg() -> str:
    path = shutil.which("ffmpeg")
    if not path:
        raise AudioError("ffmpeg is not installed or not on PATH.")
    return path


def _run(args: list[str], timeout: int = 1800) -> subprocess.CompletedProcess:
    result = subprocess.run(  # noqa: S603 - fixed arguments, no shell
        args, capture_output=True, timeout=timeout, check=False
    )
    if result.returncode != 0:
        message = result.stderr.decode("utf-8", "replace").strip().splitlines()
        raise AudioError(message[-1] if message else "ffmpeg failed")
    return result


def decode(path: Path, sample_rate: int) -> np.ndarray:
    """The first audio stream as `[channels, samples]` float at `sample_rate`."""
    with tempfile.TemporaryDirectory(prefix="mujin-decode-") as folder:
        wav = Path(folder) / "decoded.wav"
        _run(
            [
                ffmpeg(),
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-i",
                str(path),
                "-map",
                "0:a:0",
                "-c:a",
                "pcm_f32le",
                str(wav),
            ]
        )
        data, rate = sf.read(str(wav), dtype="float64", always_2d=True)
    audio = data.T
    if audio.shape[1] == 0:
        raise AudioError("the file has no audio")
    return resample(audio, int(rate), sample_rate)


def has_cover(path: Path) -> bool:
    probe = shutil.which("ffprobe")
    if not probe:
        return False
    result = subprocess.run(  # noqa: S603 - fixed arguments, no shell
        [
            probe,
            "-v",
            "error",
            "-select_streams",
            "v",
            "-show_entries",
            "stream=index",
            "-of",
            "csv=p=0",
            str(path),
        ],
        capture_output=True,
        timeout=60,
        check=False,
    )
    return result.returncode == 0 and bool(result.stdout.strip())


def write_flac(audio: np.ndarray, sample_rate: int, source: Path, target: Path, tag: str) -> None:
    """24-bit FLAC with the source's tags and cover, plus `MUJIN_ENHANCED=<tag>`.

    The output is scaled down (never clipped) if the model went past full
    scale. `target` must not exist: the worker never overwrites a file.
    """
    if target.exists():
        raise AudioError(f"{target} already exists")
    peak = float(np.max(np.abs(audio))) if audio.size else 0.0
    if peak > 0.999:
        audio = audio * (0.999 / peak)
    with tempfile.TemporaryDirectory(prefix="mujin-encode-") as folder:
        wav = Path(folder) / "enhanced.wav"
        sf.write(str(wav), audio.T.astype(np.float32), sample_rate, subtype="FLOAT")
        args = [
            ffmpeg(),
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-n",
            "-i",
            str(wav),
            "-i",
            str(source),
            "-map",
            "0:a:0",
        ]
        if has_cover(source):
            args += ["-map", "1:v:0", "-c:v", "copy", "-disposition:v:0", "attached_pic"]
        args += [
            "-map_metadata",
            "1",
            "-metadata",
            f"MUJIN_ENHANCED={tag}",
            "-c:a",
            "flac",
            "-sample_fmt",
            "s32",
            "-bits_per_raw_sample",
            "24",
            "-ar",
            str(sample_rate),
            str(target),
        ]
        _run(args)
