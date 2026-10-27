import subprocess
import time
from pathlib import Path

import numpy as np
import pytest
import soundfile as sf
from fastapi.testclient import TestClient

from mujin_enhancer.config import Config, load_or_create_secret
from mujin_enhancer.models.passthrough import PassthroughModel
from mujin_enhancer.server import Worker, create_app


class Halving(PassthroughModel):
    name = "stub"
    version = "9"
    chunk = 44_100
    overlap = 4_410

    def enhance(self, audio):
        return np.asarray(audio, dtype=np.float64) * 0.5


@pytest.fixture
def client(tmp_path: Path):
    music = tmp_path / "music"
    music.mkdir()
    config = Config(home=tmp_path / "home", shared_roots=[music])
    secret, created = load_or_create_secret(config)
    assert created and len(secret) >= 32
    assert load_or_create_secret(config) == (secret, False)
    worker = Worker(config, Halving(), secret, realtime_factor=0.1)
    app = create_app(worker)
    with TestClient(app) as test_client:
        test_client.headers["Authorization"] = f"Bearer {secret}"
        yield test_client, music, worker


def test_health_is_open_and_everything_else_needs_the_secret(client):
    test_client, _, _ = client
    anonymous = TestClient(test_client.app)
    assert anonymous.get("/v1/health").json() == {"ok": True}
    assert anonymous.get("/v1/info").status_code == 401
    wrong = anonymous.get("/v1/info", headers={"Authorization": "Bearer nope"})
    assert wrong.status_code == 401
    info = test_client.get("/v1/info").json()
    assert info["model"] == "stub"
    assert info["tag"] == "stub-9"
    assert info["sample_rate"] == 44_100
    assert info["realtime_factor"] == 0.1


def test_real_time_blocks_come_back_enhanced_and_the_same_length(client):
    test_client, _, _ = client
    block = (np.ones(4_410 * 2, dtype="<f4") * 0.8).tobytes()
    response = test_client.post(
        "/v1/realtime", content=block, headers={"X-Channels": "2", "X-Sample-Rate": "44100"}
    )
    assert response.status_code == 200
    out = np.frombuffer(response.content, dtype="<f4")
    assert len(out) == 4_410 * 2
    assert np.allclose(out, 0.4)
    # Another rate is resampled to the model's and back.
    other = (np.ones(4_800, dtype="<f4") * 0.8).tobytes()
    response = test_client.post(
        "/v1/realtime", content=other, headers={"X-Channels": "1", "X-Sample-Rate": "48000"}
    )
    assert len(np.frombuffer(response.content, dtype="<f4")) == 4_800
    bad = test_client.post("/v1/realtime", content=b"abc", headers={"X-Channels": "2"})
    assert bad.status_code == 400


def test_paths_outside_the_shared_folders_are_refused(client, tmp_path: Path):
    test_client, music, _ = client
    secret_file = tmp_path / "home" / "secret.txt"
    refused = test_client.post("/v1/jobs", json={"path": str(secret_file)})
    assert refused.status_code == 403
    sneaky = test_client.post("/v1/jobs", json={"path": str(music / ".." / "home" / "secret.txt")})
    assert sneaky.status_code == 403
    missing = test_client.post("/v1/jobs", json={"path": str(music / "nothing.mp3")})
    assert missing.status_code == 404


def wait_for(test_client, job_id: str) -> dict:
    for _ in range(300):
        job = test_client.get(f"/v1/jobs/{job_id}").json()
        if job["state"] in ("done", "failed", "cancelled"):
            return job
        time.sleep(0.05)
    raise AssertionError("the job did not finish")


def test_an_uploaded_file_becomes_a_tagged_24_bit_flac(client, tmp_path: Path):
    test_client, _, _ = client
    source = tmp_path / "song.mp3"
    subprocess.run(
        [
            "ffmpeg",
            "-nostdin",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=3:sample_rate=48000",
            "-ac",
            "2",
            "-metadata",
            "title=Architects",
            "-metadata",
            "artist=Rise Against",
            "-c:a",
            "libmp3lame",
            "-b:a",
            "128k",
            str(source),
        ],
        check=True,
    )
    original = source.read_bytes()
    job = test_client.post(
        "/v1/jobs?filename=song.mp3",
        content=original,
        headers={"Content-Type": "application/octet-stream"},
    ).json()
    done = wait_for(test_client, job["id"])
    assert done["state"] == "done", done
    result = test_client.get(f"/v1/jobs/{job['id']}/result")
    assert result.status_code == 200
    output = tmp_path / "out.flac"
    output.write_bytes(result.content)
    info = sf.info(str(output))
    assert info.samplerate == 44_100
    assert info.subtype == "PCM_24"
    tags = subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "format_tags", "-of", "json", str(output)],
        capture_output=True,
        check=True,
        text=True,
    ).stdout.lower()
    assert '"mujin_enhanced": "stub-9"' in tags
    assert '"title": "architects"' in tags
    # The source was only read.
    assert source.read_bytes() == original
    assert test_client.delete(f"/v1/jobs/{job['id']}").status_code == 204
    assert test_client.get(f"/v1/jobs/{job['id']}").status_code == 404


def test_a_shared_path_is_read_in_place(client):
    test_client, music, _ = client
    source = music / "song.wav"
    sf.write(str(source), np.zeros((44_100, 2)), 44_100, subtype="PCM_16")
    job = test_client.post("/v1/jobs", json={"path": str(source)}).json()
    finished = wait_for(test_client, job["id"])
    assert finished["state"] == "done", finished
    assert source.exists(), "never deleted"
