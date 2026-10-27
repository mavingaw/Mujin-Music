"""Restoration models behind one interface.

Only Apollo is implemented. NVIDIA A2SB and AudioSR are documented
alternatives (see the README) that would plug in behind the same interface.
"""

from __future__ import annotations

from typing import Protocol

import numpy as np


class Model(Protocol):
    #: Short name written into files, e.g. `apollo`.
    name: str
    version: str
    #: The rate the model works at.
    sample_rate: int
    #: `cuda` or `cpu`.
    device: str
    device_name: str
    license: str
    #: Samples per chunk the model handles at once, and the crossfade and
    #: context padding around each chunk.
    chunk: int
    overlap: int
    pad: int

    def enhance(self, audio: np.ndarray) -> np.ndarray:
        """`[channels, samples]` float in, the same shape out."""
        ...


#: Models that could sit behind the same interface. Only Apollo ships.
DOCUMENTED_ALTERNATIVES = {
    "a2sb": "NVIDIA A2SB (Audio-to-Audio Schrodinger Bridge): higher quality, about 7 GB of "
    "weights, NVIDIA non-commercial licence: personal use only. Not implemented.",
    "audiosr": "AudioSR (versatile audio super-resolution, 48 kHz): diffusion based and slow. "
    "Not implemented.",
}


def load_model(name: str, device: str, cache_dir, offline: bool) -> Model:
    if name == "apollo":
        from .apollo import ApolloModel

        return ApolloModel.load(device=device, cache_dir=cache_dir, offline=offline)
    if name == "passthrough":
        from .passthrough import PassthroughModel

        return PassthroughModel()
    if name in DOCUMENTED_ALTERNATIVES:
        raise SystemExit(DOCUMENTED_ALTERNATIVES[name])
    raise SystemExit(f"Unknown model {name!r}. Use apollo.")
