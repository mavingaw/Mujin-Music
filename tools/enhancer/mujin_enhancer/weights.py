"""The Apollo checkpoint: pinned, verified, and fetched at most once.

The checkpoint is a pickled PyTorch file, so it is only ever loaded from the
pinned Hugging Face revision below and only after its SHA-256 matched.
Place the file by hand (same name, same hash) to run fully offline.
"""

from __future__ import annotations

import hashlib
import os
import shutil
import urllib.request
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class PinnedWeights:
    repo: str
    revision: str
    filename: str
    sha256: str
    size: int
    license: str

    @property
    def url(self) -> str:
        return f"https://huggingface.co/{self.repo}/resolve/{self.revision}/{self.filename}"

    def path_in(self, cache_dir: Path) -> Path:
        """`<cache>/apollo/<revision>/pytorch_model.bin`."""
        return cache_dir / "apollo" / self.revision / self.filename


#: JusperLee/Apollo on Hugging Face, checked on 2026-10-27: the only
#: checkpoint in the repository, licensed CC BY-SA 4.0.
APOLLO = PinnedWeights(
    repo="JusperLee/Apollo",
    revision="c68bd80fdd9c0d93d2f4a833cb154624f660a561",
    filename="pytorch_model.bin",
    sha256="99d9af7f1ff20e63c393035513a655392818d66b4d7fc23d658175c1f15e8d76",
    size=66_541_845,
    license="CC BY-SA 4.0",
)


class WeightsError(RuntimeError):
    pass


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def ensure_weights(
    pinned: PinnedWeights,
    cache_dir: Path,
    *,
    offline: bool,
    opener=urllib.request.urlopen,
) -> Path:
    """The verified checkpoint, downloading it once when allowed."""
    target = pinned.path_in(cache_dir)
    if target.is_file():
        actual = sha256_of(target)
        if actual != pinned.sha256:
            raise WeightsError(
                f"{target} does not match the pinned SHA-256 (expected {pinned.sha256}, "
                f"found {actual}). Delete it and let the worker download it again, or "
                "place the exact file from the pinned revision."
            )
        return target
    if offline:
        raise WeightsError(
            f"The model weights are missing and the worker is offline. Download {pinned.url} "
            f"once on any computer and place it at {target} (SHA-256 {pinned.sha256})."
        )
    target.parent.mkdir(parents=True, exist_ok=True)
    partial = target.with_suffix(target.suffix + ".part")
    print(f"Downloading the {pinned.repo} weights ({pinned.size / 1e6:.0f} MB), once...")
    request = urllib.request.Request(pinned.url, headers={"User-Agent": "mujin-enhancer"})  # noqa: S310
    with opener(request) as response, partial.open("wb") as handle:  # noqa: S310
        shutil.copyfileobj(response, handle, length=1024 * 1024)
    actual = sha256_of(partial)
    if actual != pinned.sha256:
        partial.unlink(missing_ok=True)
        raise WeightsError(
            f"The downloaded file does not match the pinned SHA-256 ({actual}); it was deleted."
        )
    os.replace(partial, target)
    return target
