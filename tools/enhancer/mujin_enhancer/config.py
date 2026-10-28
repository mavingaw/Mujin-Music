"""Where the worker keeps its secret and model weights, and its options."""

from __future__ import annotations

import contextlib
import os
import secrets
import sys
from dataclasses import dataclass, field
from pathlib import Path


def default_home() -> Path:
    """`%LOCALAPPDATA%\\MujinEnhancer` on Windows, else `~/.local/share/mujin-enhancer`."""
    override = os.environ.get("MUJIN_ENHANCER_HOME")
    if override:
        return Path(override)
    if sys.platform == "win32":
        base = os.environ.get("LOCALAPPDATA") or str(Path.home() / "AppData" / "Local")
        return Path(base) / "MujinEnhancer"
    base = os.environ.get("XDG_DATA_HOME") or str(Path.home() / ".local" / "share")
    return Path(base) / "mujin-enhancer"


@dataclass
class Config:
    home: Path = field(default_factory=default_home)
    host: str = "0.0.0.0"  # noqa: S104 - the worker serves the LAN on purpose
    port: int = 8765
    model: str = "apollo"
    device: str = "auto"
    #: Never download anything (weights must already be in the cache).
    offline: bool = False
    #: Folders the worker may read by path (when it sees Mujin's disk).
    shared_roots: list[Path] = field(default_factory=list)
    #: Largest upload accepted, in bytes.
    max_upload: int = 1024 * 1024 * 1024

    @property
    def cache_dir(self) -> Path:
        return self.home / "models"

    @property
    def jobs_dir(self) -> Path:
        return self.home / "jobs"

    @property
    def secret_file(self) -> Path:
        return self.home / "secret.txt"


def load_or_create_secret(config: Config) -> tuple[str, bool]:
    """The shared secret Mujin Music sends; created on the first run.

    Returns the secret and whether it was just created.
    """
    path = config.secret_file
    if path.is_file():
        value = path.read_text(encoding="utf-8").strip()
        if len(value) >= 32:
            return value, False
    path.parent.mkdir(parents=True, exist_ok=True)
    value = secrets.token_urlsafe(32)
    path.write_text(value + "\n", encoding="utf-8")
    with contextlib.suppress(OSError):
        path.chmod(0o600)
    return value, True
