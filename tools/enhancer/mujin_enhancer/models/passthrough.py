"""A stand-in model for tests and for trying the plumbing without a GPU.

It returns its input unchanged, so files it "enhances" sound exactly like
the originals. Never use it for real listening.
"""

from __future__ import annotations

import numpy as np


class PassthroughModel:
    name = "passthrough"
    version = "0"
    sample_rate = 44_100
    device = "cpu"
    device_name = "CPU (passthrough test model)"
    license = "GPL-3.0-only"
    chunk = 44_100 * 10
    overlap = 44_100
    pad = 0

    def enhance(self, audio: np.ndarray) -> np.ndarray:
        return np.array(audio, dtype=np.float64, copy=True)
