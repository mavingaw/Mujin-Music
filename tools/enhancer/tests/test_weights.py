import hashlib
import io
from pathlib import Path

import pytest

from mujin_enhancer.weights import APOLLO, PinnedWeights, WeightsError, ensure_weights


def pinned(data: bytes) -> PinnedWeights:
    return PinnedWeights(
        repo="example/model",
        revision="0" * 40,
        filename="pytorch_model.bin",
        sha256=hashlib.sha256(data).hexdigest(),
        size=len(data),
        license="test",
    )


def test_the_apollo_pin_names_the_exact_revision_file_and_hash():
    assert APOLLO.repo == "JusperLee/Apollo"
    assert APOLLO.filename == "pytorch_model.bin"
    assert len(APOLLO.revision) == 40
    assert len(APOLLO.sha256) == 64
    assert APOLLO.url.startswith(
        "https://huggingface.co/JusperLee/Apollo/resolve/c68bd80fdd9c0d93d2f4a833cb154624f660a561/"
    )
    assert APOLLO.license == "CC BY-SA 4.0"


def test_offline_without_weights_explains_where_to_put_them(tmp_path: Path):
    with pytest.raises(WeightsError) as error:
        ensure_weights(APOLLO, tmp_path, offline=True)
    assert str(APOLLO.path_in(tmp_path)) in str(error.value)
    assert APOLLO.sha256 in str(error.value)


def test_a_manually_placed_file_is_used_when_its_hash_matches(tmp_path: Path):
    data = b"weights"
    pin = pinned(data)
    target = pin.path_in(tmp_path)
    target.parent.mkdir(parents=True)
    target.write_bytes(data)

    def no_network(*_args, **_kwargs):
        raise AssertionError("must not download")

    assert ensure_weights(pin, tmp_path, offline=False, opener=no_network) == target
    target.write_bytes(b"tampered")
    with pytest.raises(WeightsError):
        ensure_weights(pin, tmp_path, offline=True)


def test_a_download_is_verified_before_it_is_used(tmp_path: Path):
    data = b"real weights"
    pin = pinned(data)
    path = ensure_weights(pin, tmp_path, offline=False, opener=lambda _r: io.BytesIO(data))
    assert path.read_bytes() == data
    other = pinned(b"expected")
    with pytest.raises(WeightsError):
        ensure_weights(other, tmp_path / "x", offline=False, opener=lambda _r: io.BytesIO(b"evil"))
    assert not list((tmp_path / "x").rglob("*.part"))
    assert not other.path_in(tmp_path / "x").exists()
