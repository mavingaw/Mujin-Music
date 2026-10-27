# Better versions and AI enhance

Mujin Music can make your collection sound better in two ways:

1. **Better versions** finds albums you only have as MP3s or other lossy
   files (and "lossless" files that were really made from MP3s), and asks
   **FETCH**, your download manager, for a real lossless copy. When the copy
   arrives and checks out, it plays instead. Your old copy stays.
2. **AI enhance** sends lossy tracks to the **enhancer**, a small program on
   your PC with the graphics card. It uses the Apollo model to restore some
   of what MP3 compression threw away, and saves a new copy. It can also do
   this live while you listen (experimental).

Everything runs on your own machines: the Mujin Music server, FETCH and the
enhancer. Nothing goes to a cloud service.

## What to expect from each

|                | A lossless copy from FETCH   | An AI-enhanced copy                                                      |
| -------------- | ---------------------------- | ------------------------------------------------------------------------ |
| What it is     | The real CD or hi-res master | Your MP3, with treble the AI _imagined_                                  |
| Labelled       | Lossless (copper)            | AI-enhanced (blue), never "lossless"                                     |
| Plays first?   | Always                       | Only if no lossless copy exists, and "Prefer AI-enhanced versions" is on |
| Original kept? | Yes, as an alternate version | Yes, one click away                                                      |

AI enhancement makes plausible high frequencies. It does not and cannot
bring back the original recording. Think of it as a good-sounding guess.

## Connect FETCH

You need: FETCH running on your network, and a token for Mujin Music.

1. **Get a token for Mujin Music from FETCH.** FETCH allows one agent token
   per app. Ask FETCH for a separate token for Mujin Music. Do **not**
   generate a new token for an app that already has one: that signs the
   other app out (for example the voice assistant).
2. In Mujin Music, open **Settings → Better versions**.
3. Under **FETCH**, turn it on and check the address. The default is
   `http://192.168.1.58:8490`.
4. Paste the token into **Agent token** and press **Save**. Mujin Music
   stores it encrypted on the server and never shows it again; you only
   see "Saved" and a **Replace** button.
5. Under **FETCH saves music into this library folder**, pick the library
   folder that is FETCH's `Music` share (`\\192.168.1.58\Music`). Then
   Mujin Music only rescans that folder when an album arrives. If you leave
   it on "None", it rescans everything, which is slower.
6. Press **Save**, then **Check connection**. You should see "Connected",
   FETCH's version, and "every path Mujin Music uses is there". If a path
   is missing, update FETCH.

The Cloudflare Access fields are only for reaching FETCH through its public
address. On your home network, leave them empty.

## Use Better versions

Open **Library → Library tools → Better versions**.

- Each album shows why it is listed: **MP3 320**, **Low bitrate**,
  **Fake lossless · 16 kHz** (a FLAC made from an MP3), or **AI-enhanced
  only**. Hover a badge for details.
- **Check for fake lossless** scans your FLAC, ALAC and WAV files. A real CD
  has music up to about 20–22 kHz; a file made from an MP3 stops sharply at
  16–20 kHz. Quiet or very old recordings are reported as "cannot tell"
  rather than accused.
- The magnifier asks FETCH whether any of its sources has the album, before
  you request it.
- **Find lossless** asks FETCH for one album. Tick several and press
  **Request selected** for many; you see the exact text sent to FETCH
  before you confirm, for example
  `the album Endgame by Rise Against (2011) as flac`.
- While FETCH works, the album shows **Lossless version on its way · 45%**,
  here and on the album page. If FETCH passes it to Lidarr or another app,
  it says **Handed to Lidarr**.
- When FETCH is done, Mujin Music rescans, matches the new files to your
  album (MusicBrainz ids first, else artist, album, track numbers and track
  lengths), and runs the fake-lossless check on them.
  - Real lossless: it becomes the version that plays. Your old copy stays
    as an alternate version under **Album versions**.
  - Fake lossless, or not the same album: you see why, and your original
    keeps playing.

Mujin Music never requests anything by itself unless you turn on **Request
automatically** (off by default). Then it asks for one album at a time, at
most the number per day you set.

Optional: **Move replaced lossy files** moves the old MP3s into a folder you
choose after a verified upgrade. Nothing is ever deleted.

Listeners (non-admin profiles) see the status on album pages; only admins
can request, enhance or change settings.

## Start the enhancer on Windows

You need: a Windows PC (an NVIDIA graphics card makes it fast), about 4 GB
of free disk space, and the Mujin Music source folder `tools/enhancer`.

1. Install **uv** once. Open PowerShell and run:
   `powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"`
2. Install **ffmpeg**: `winget install Gyan.FFmpeg`
3. Double-click **`start-enhancer.cmd`** in `tools/enhancer`.

The first start takes a while: it installs PyTorch, downloads the Apollo
model once (67 MB), and measures how fast your PC is. Then it shows a
**shared secret**. Keep the window open.

4. In Mujin Music, **Settings → Better versions → AI enhancer**: turn it
   on, enter the PC's address (for example `http://192.168.1.58:8765`),
   paste the shared secret, **Save**, and **Check connection**. You should
   see "Apollo 1.0 on" your graphics card, and whether it is fast enough
   for real time.
5. When Windows asks about the firewall, allow private networks.

To run it fully offline, place the model file by hand; the exact file name,
folder and SHA-256 are in `tools/enhancer/README.md`, then start it with
`start-enhancer.cmd --offline`. After that it never connects to the
internet.

## Run the enhancer in Docker

If your GPU machine runs Docker (for example next to the Mujin Music server
container), see `tools/enhancer/docker-compose.example.yml`:

- **NVIDIA GPU**: `docker build -f Dockerfile.cuda -t mujin-enhancer:cuda tools/enhancer`
  and run with `--gpus all` (needs the NVIDIA Container Toolkit).
- **CPU only**: `docker build -f Dockerfile -t mujin-enhancer:cpu tools/enhancer`.
  It works, slowly, and cannot do real time.

`docker logs` shows the shared secret on first start. If the enhancer
container can read your music at the same path as the Mujin Music server,
turn on **The enhancer can read the music folders** so files are not
uploaded.

The Mujin Music server needs a folder it may write to for enhanced files.
In Docker the music is usually mounted read-only, so mount a writable folder
(for example `/path/to/enhanced:/enhanced`) and set **Save AI-enhanced files
in** to `/enhanced`. Mujin Music adds that folder to your library by itself.

## Use AI enhance

- **Enhance** on an album in Better versions makes AI-enhanced copies of its
  lossy tracks.
- **Enhance all lossy that FETCH couldn't find** queues every album FETCH
  could not get. The queue runs in the background and continues after a
  restart. **Stop** empties it.
- New files go into the enhanced folder (by default "Mujin Enhanced" in your
  first library folder) as 24-bit FLAC with the original tags, cover, and a
  `MUJIN_ENHANCED=apollo-1.0` tag. Originals are never changed.
- The album page shows **AI-enhanced · 9 of 10 tracks**. Admins can press
  **Play the originals** to switch back for that album.
- **Prefer AI-enhanced versions** (on by default) decides whether enhanced
  copies play instead of the lossy originals. A real lossless copy always
  plays first.

### In real time (experimental)

Open the **DSP engine** for a zone that plays on the server (a DAC, a Mujin
Bridge, a network player) and turn on **AI enhance in real time**. The
track streams through the enhancer a few seconds ahead of what you hear,
in 4-second pieces blended together. If the enhancer falls behind, the
original plays at once, with no gap. It only works on lossy tracks at
44.1 kHz, and only when the enhancer needs less than half a second per
second of audio. The signal path then shows **AI-enhanced (Apollo)** in
blue.

Right next to it, **Upsampling (sample rate conversion)** converts to your
DAC's best rate. That is resampling, not enhancement: it adds no detail.

## Licences

Apollo (the model and its weights) is by Kai Li and Yi Luo and is licensed
CC BY-SA 4.0. NVIDIA A2SB (higher quality, about 7 GB, personal
non-commercial use only) and AudioSR are possible alternatives behind the
same interface, but are not included.
