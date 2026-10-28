# Mujin Music enhancer

The AI enhancer worker for Mujin Music's **Better versions**. You run it on
your own computer with the GPU (Windows or Linux, or in Docker). Mujin Music
sends it lossy tracks (MP3, AAC and the like) and gets back restored copies.

**What it does, honestly.** The default model, Apollo, was trained to undo
MP3 compression. It _generates_ plausible high frequencies and detail that
the encoder threw away. It cannot recover the original master. Mujin Music
labels everything it makes **AI-enhanced** (in the enhanced colour), never
"lossless" or "hi-res", keeps the original file, and always prefers a real
lossless copy (for example one from FETCH) over an AI-enhanced one.

Everything stays on your machines. After the one-time model download (or a
manual copy of the weights) the worker never contacts the internet.

## Start it on Windows

1. Install [uv](https://docs.astral.sh/uv/) once:
   `powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"`
2. Install [ffmpeg](https://ffmpeg.org/download.html) and make sure `ffmpeg`
   works in a new Command Prompt (for example `winget install Gyan.FFmpeg`).
3. Double-click `start-enhancer.cmd` in this folder.

The first start installs Python packages (PyTorch with CUDA is about 2.5 GB),
downloads the Apollo weights once (67 MB), measures the speed of your GPU and
prints the **shared secret**. Copy it into Mujin Music: **Settings → Better
versions → AI enhancer**, together with this computer's address, for example
`http://192.168.1.58:8765`. Allow the port through Windows Firewall when
Windows asks (private networks only).

Options go after the script name, for example
`start-enhancer.cmd --port 8766 --shared-root "\\192.168.1.58\Music"`.

## Start it in Docker

Next to the Mujin Music server container (see `docker-compose.example.yml` in
this folder):

```sh
# NVIDIA GPU (needs the NVIDIA Container Toolkit)
docker build -f Dockerfile.cuda -t mujin-enhancer:cuda .
docker run -d --gpus all -p 8765:8765 -v mujin-enhancer:/data mujin-enhancer:cuda

# CPU only (works, but slowly; no real time)
docker build -f Dockerfile -t mujin-enhancer:cpu .
docker run -d -p 8765:8765 -v mujin-enhancer:/data mujin-enhancer:cpu
```

`docker logs <container>` shows the shared secret on the first start. Mount
your music read-only and pass `--shared-root /music` (and tick "The enhancer
can read the music folders" in Mujin Music) to skip uploads.

## Fully offline

The worker downloads exactly one file, once, from a pinned revision:

|          |                                                                                                              |
| -------- | ------------------------------------------------------------------------------------------------------------ |
| Model    | Apollo, `JusperLee/Apollo` on Hugging Face                                                                   |
| Revision | `c68bd80fdd9c0d93d2f4a833cb154624f660a561`                                                                   |
| File     | `pytorch_model.bin` (66,541,845 bytes)                                                                       |
| SHA-256  | `99d9af7f1ff20e63c393035513a655392818d66b4d7fc23d658175c1f15e8d76`                                           |
| Licence  | CC BY-SA 4.0 (model card and GitHub repository)                                                              |
| URL      | `https://huggingface.co/JusperLee/Apollo/resolve/c68bd80fdd9c0d93d2f4a833cb154624f660a561/pytorch_model.bin` |

To never connect at all, download that file on any computer and place it at

- Windows: `%LOCALAPPDATA%\MujinEnhancer\models\apollo\c68bd80fdd9c0d93d2f4a833cb154624f660a561\pytorch_model.bin`
- Linux: `~/.local/share/mujin-enhancer/models/apollo/c68bd80fdd9c0d93d2f4a833cb154624f660a561/pytorch_model.bin`
- Docker: `/data/models/apollo/c68bd80fdd9c0d93d2f4a833cb154624f660a561/pytorch_model.bin` in the volume

then start with `--offline` (or `MUJIN_ENHANCER_OFFLINE=1`). The worker checks
the SHA-256 before every load. The checkpoint is a pickled PyTorch file, so it
is only loaded when the hash matches this pinned revision.

## Speed and real time

On start the worker measures its real-time factor (seconds of processing per
second of audio). Mujin Music allows "AI enhance in real time" only when it is
below 0.5, so the worker can stay ahead of playback. A recent NVIDIA GPU is
typically far below that; a CPU is not. Batch enhancing works at any speed.

## Models

Only **Apollo** is implemented. The worker is pluggable (`mujin_enhancer/
models/`), and these would fit behind the same interface:

- **NVIDIA A2SB** (Audio-to-Audio Schrödinger Bridge): higher quality, about
  7 GB of weights, NVIDIA's non-commercial licence: personal use only.
- **AudioSR**: diffusion-based super-resolution to 48 kHz; slow.

`--model passthrough` runs a do-nothing model, for testing the plumbing
without a GPU. Never use it for listening: it changes nothing.

## HTTP API

Every call except health needs `Authorization: Bearer <shared secret>`.

| Call                           | What it does                                                                                   |
| ------------------------------ | ---------------------------------------------------------------------------------------------- |
| `GET /v1/health`               | `{"ok": true}`                                                                                 |
| `GET /v1/info`                 | model, version, tag (`apollo-1.0`), device, real-time factor, licence                          |
| `POST /v1/jobs?filename=x.mp3` | body: the file's bytes; `202 {"id", "state"}`                                                  |
| `POST /v1/jobs`                | `{"path": "..."}`, a file under a `--shared-root`                                              |
| `GET /v1/jobs/{id}`            | `{"state": queued/running/done/failed/cancelled, "progress": 0..1}`                            |
| `GET /v1/jobs/{id}/result`     | the 24-bit FLAC                                                                                |
| `DELETE /v1/jobs/{id}`         | cancels and deletes the job's files                                                            |
| `POST /v1/realtime`            | raw `float32` interleaved PCM in (`X-Sample-Rate`, `X-Channels`), the same length enhanced out |

Results are 24-bit FLAC at 44.1 kHz (Apollo's rate), with the source's tags
and cover art and an extra tag `MUJIN_ENHANCED=apollo-1.0`. The worker never
writes to your music: uploads and results live in its own `jobs` folder and
are deleted once Mujin Music has fetched them.

## Development

```sh
uv run --extra test pytest       # tests use a stub model; no weights, no GPU
uvx ruff check . && uvx ruff format --check .
```

## Licences

The worker is part of Mujin Music (GPL-3.0-only). `mujin_enhancer/models/
apollo.py` adapts the Apollo network definition by Kai Li and Yi Luo
(Look2Hear, https://github.com/JusperLee/Apollo), licensed CC BY-SA 4.0,
which Creative Commons lists as one-way compatible with GPLv3. The Apollo
weights are not in this repository; they are CC BY-SA 4.0 too. If you share
audio the model made, credit Apollo.
