"""The worker's HTTP API (see the README for the contract).

Every route except `/v1/health` needs `Authorization: Bearer <secret>`.
Jobs run one at a time on the model; real-time blocks are served between
a job's chunks, so live playback never waits for a whole album.
"""

from __future__ import annotations

import hmac
import queue
import re
import shutil
import threading
import time
import uuid
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np
from fastapi import Depends, FastAPI, HTTPException, Request
from fastapi.responses import FileResponse, JSONResponse, Response
from starlette.concurrency import run_in_threadpool

from . import __version__
from .audio import AudioError, decode, write_flac
from .config import Config
from .dsp import process_in_chunks, resample

#: Largest real-time block: 30 s of stereo float.
MAX_REALTIME_BYTES = 30 * 48_000 * 2 * 4
#: Finished jobs kept for download.
KEEP_FINISHED = 64


@dataclass
class Job:
    id: str
    source: Path
    uploaded: bool
    state: str = "queued"
    progress: float = 0.0
    message: str = ""
    output: Path | None = None
    created: float = field(default_factory=time.time)
    cancelled: threading.Event = field(default_factory=threading.Event)

    def view(self) -> dict:
        return {
            "id": self.id,
            "state": self.state,
            "progress": round(self.progress, 4),
            "message": self.message,
        }


class Worker:
    """The model, the job queue and the lock that shares the GPU."""

    def __init__(self, config: Config, model, secret: str, realtime_factor: float | None):
        self.config = config
        self.model = model
        self.secret = secret
        self.realtime_factor = realtime_factor
        self.lock = threading.Lock()
        self.jobs: dict[str, Job] = {}
        self.queue: queue.Queue[str] = queue.Queue()
        config.jobs_dir.mkdir(parents=True, exist_ok=True)
        self.thread = threading.Thread(target=self._run, name="enhance-jobs", daemon=True)
        self.thread.start()

    @property
    def tag(self) -> str:
        return f"{self.model.name}-{self.model.version}"

    def _model_call(self, audio: np.ndarray) -> np.ndarray:
        with self.lock:
            return self.model.enhance(audio)

    def enhance(self, audio: np.ndarray, progress=None, cancelled=None) -> np.ndarray:
        def call(chunk: np.ndarray) -> np.ndarray:
            if cancelled is not None and cancelled.is_set():
                raise InterruptedError("cancelled")
            return self._model_call(chunk)

        return process_in_chunks(
            audio, call, self.model.chunk, self.model.overlap, self.model.pad, progress
        )

    def submit(self, source: Path, uploaded: bool) -> Job:
        job = Job(id=uuid.uuid4().hex, source=source, uploaded=uploaded)
        (self.config.jobs_dir / job.id).mkdir(parents=True, exist_ok=True)
        self.jobs[job.id] = job
        self._prune()
        self.queue.put(job.id)
        return job

    def _prune(self) -> None:
        finished = [
            job for job in self.jobs.values() if job.state in ("done", "failed", "cancelled")
        ]
        finished.sort(key=lambda job: job.created)
        for job in finished[: max(0, len(finished) - KEEP_FINISHED)]:
            self.forget(job.id)

    def forget(self, job_id: str) -> None:
        job = self.jobs.pop(job_id, None)
        if job is None:
            return
        job.cancelled.set()
        shutil.rmtree(self.config.jobs_dir / job.id, ignore_errors=True)

    def _run(self) -> None:
        while True:
            job_id = self.queue.get()
            job = self.jobs.get(job_id)
            if job is None or job.cancelled.is_set():
                continue
            job.state = "running"
            folder = self.config.jobs_dir / job.id
            try:
                audio = decode(job.source, self.model.sample_rate)

                def progress(value: float, job: Job = job) -> None:
                    job.progress = value

                enhanced = self.enhance(audio, progress, job.cancelled)
                output = folder / "enhanced.flac"
                write_flac(enhanced, self.model.sample_rate, job.source, output, self.tag)
                job.output = output
                job.progress = 1.0
                job.state = "done"
            except InterruptedError:
                job.state = "cancelled"
            except (AudioError, RuntimeError, ValueError, OSError) as error:
                job.state = "failed"
                job.message = str(error)[:500]
            finally:
                if job.uploaded:
                    shutil.rmtree(job.source.parent, ignore_errors=True)


def create_app(worker: Worker) -> FastAPI:
    app = FastAPI(title="Mujin Music enhancer", version=__version__, docs_url=None, redoc_url=None)
    config = worker.config

    def authorized(request: Request) -> None:
        header = request.headers.get("authorization", "")
        token = header[7:] if header.lower().startswith("bearer ") else ""
        if not token or not hmac.compare_digest(token.encode(), worker.secret.encode()):
            raise HTTPException(status_code=401, detail="Wrong or missing shared secret.")

    def find(job_id: str) -> Job:
        job = worker.jobs.get(job_id)
        if job is None:
            raise HTTPException(status_code=404, detail="No such job.")
        return job

    @app.get("/v1/health")
    def health() -> dict:
        return {"ok": True}

    @app.get("/v1/info", dependencies=[Depends(authorized)])
    def info() -> dict:
        model = worker.model
        return {
            "worker_version": __version__,
            "model": model.name,
            "model_version": model.version,
            "tag": worker.tag,
            "device": model.device,
            "device_name": model.device_name,
            "sample_rate": model.sample_rate,
            "realtime_factor": worker.realtime_factor,
            "license": model.license,
            "shared_roots": [str(root) for root in config.shared_roots],
        }

    @app.post("/v1/jobs", dependencies=[Depends(authorized)], status_code=202)
    async def create_job(request: Request, filename: str = "track") -> dict:
        if request.headers.get("content-type", "").startswith("application/json"):
            body = await request.json()
            raw = body.get("path") if isinstance(body, dict) else None
            if not isinstance(raw, str) or not raw:
                raise HTTPException(status_code=400, detail="Send a path or the file.")
            path = Path(raw).resolve()
            if not any(path.is_relative_to(root.resolve()) for root in config.shared_roots):
                raise HTTPException(
                    status_code=403,
                    detail="That path is outside the folders this worker may read.",
                )
            if not path.is_file():
                raise HTTPException(status_code=404, detail="No such file on this computer.")
            return worker.submit(path, uploaded=False).view()
        suffix = re.sub(r"[^A-Za-z0-9]", "", Path(filename).suffix)[:8]
        folder = config.jobs_dir / f"upload-{uuid.uuid4().hex}"
        folder.mkdir(parents=True, exist_ok=True)
        upload = folder / f"source.{suffix or 'bin'}"
        written = 0
        with upload.open("wb") as handle:
            async for chunk in request.stream():
                written += len(chunk)
                if written > config.max_upload:
                    handle.close()
                    shutil.rmtree(folder, ignore_errors=True)
                    raise HTTPException(status_code=413, detail="The file is too large.")
                handle.write(chunk)
        if written == 0:
            shutil.rmtree(folder, ignore_errors=True)
            raise HTTPException(status_code=400, detail="The upload was empty.")
        return worker.submit(upload, uploaded=True).view()

    @app.get("/v1/jobs/{job_id}", dependencies=[Depends(authorized)])
    def get_job(job_id: str) -> dict:
        return find(job_id).view()

    @app.get("/v1/jobs/{job_id}/result", dependencies=[Depends(authorized)])
    def get_result(job_id: str):
        job = find(job_id)
        if job.state != "done" or job.output is None or not job.output.is_file():
            raise HTTPException(status_code=409, detail="The job has not finished.")
        return FileResponse(job.output, media_type="audio/flac", filename="enhanced.flac")

    @app.delete("/v1/jobs/{job_id}", dependencies=[Depends(authorized)])
    def delete_job(job_id: str) -> Response:
        find(job_id)
        worker.forget(job_id)
        return Response(status_code=204)

    @app.post("/v1/realtime", dependencies=[Depends(authorized)])
    async def realtime(request: Request) -> Response:
        try:
            channels = int(request.headers.get("x-channels", "2"))
            rate = int(request.headers.get("x-sample-rate", str(worker.model.sample_rate)))
        except ValueError:
            raise HTTPException(
                status_code=400, detail="Bad X-Channels or X-Sample-Rate."
            ) from None
        if channels not in (1, 2) or not 8_000 <= rate <= 192_000:
            raise HTTPException(status_code=400, detail="Mono or stereo, 8 to 192 kHz.")
        body = await request.body()
        if not body or len(body) > MAX_REALTIME_BYTES or len(body) % (4 * channels):
            raise HTTPException(status_code=400, detail="Send whole frames of float32 PCM.")
        out = await run_in_threadpool(enhance_block, body, channels, rate)
        return Response(content=out, media_type="application/octet-stream")

    def enhance_block(body: bytes, channels: int, rate: int) -> bytes:
        frames = np.frombuffer(body, dtype="<f4").astype(np.float64)
        audio = frames.reshape(-1, channels).T
        model_rate = worker.model.sample_rate
        working = resample(audio, rate, model_rate) if rate != model_rate else audio
        enhanced = worker.enhance(working)
        if rate != model_rate:
            enhanced = resample(enhanced, model_rate, rate)
            # Exactly as many frames as were sent.
            enhanced = enhanced[:, : audio.shape[1]]
            if enhanced.shape[1] < audio.shape[1]:
                enhanced = np.pad(enhanced, ((0, 0), (0, audio.shape[1] - enhanced.shape[1])))
        return np.ascontiguousarray(enhanced.T, dtype="<f4").tobytes()

    @app.exception_handler(AudioError)
    async def audio_error(_: Request, error: AudioError) -> JSONResponse:
        return JSONResponse(status_code=422, content={"detail": str(error)})

    return app
