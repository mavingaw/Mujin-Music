# Mujin Music

Your music. Instantly.

Mujin Music is a local-first music app for your own collection. Based on
[Parson](https://github.com/ParsonLabs/Parson) by ParsonLabs.

Choose a folder. Start listening.

![Mujin Music](apps/site/public/screenshots/01-home.png)

[Download](https://parson.dev) ·
[Docs](https://parson.dev/docs) ·
[Releases](https://github.com/mavingaw/parson-jin/releases/latest)

## Start

Install Mujin Music, create your account, and choose your music folder.

Desktop apps include everything needed to run your library.

## Download

Get Windows, macOS and Linux packages from the [latest release](https://github.com/mavingaw/parson-jin/releases/latest).

The server container is `ghcr.io/parsonlabs/parson:latest`. Versioned image tags are also published for pinned deployments.

Build provenance is included with the release.

## Docker

```yaml
services:
  parson:
    image: ghcr.io/parsonlabs/parson:latest
    network_mode: host
    volumes:
      - /path/to/parson-data:/Parson
      - /path/to/music:/music:ro
    restart: unless-stopped
```

Open `http://localhost:1993`, create your account, and choose `/music`.

## Server and remotes

The Mujin Music server holds your library. Mujin Music Desktop, the Android app, and any browser are remotes for it. Mujin Music Desktop can run the server itself, or be a remote for a server on your NAS or another computer. See [docs/server.md](docs/server.md) for headless installs on Linux, NAS, Docker and Windows, and for how to connect phones.

## Better versions and AI enhance

Library tools → **Better versions** lists albums that are only lossy (or
"lossless" files made from MP3s, found by a spectral check) and asks
**FETCH**, your own download manager, for real lossless copies. Verified
copies play instead; the old copy stays as an alternate version. The
optional **AI enhancer** (`tools/enhancer`, Apollo on your GPU, Windows or
Docker) makes AI-enhanced copies of what FETCH cannot find, and can enhance
lossy tracks live on server zones. AI-enhanced audio is always labelled as
such, never as lossless. See
[docs/better-versions.md](docs/better-versions.md).

## Privacy

Mujin Music does not upload analytics or crash reports.

Music, accounts, playlists, history, and recommendations stay on infrastructure
you control. External requests are limited to services you use, such as lyrics,
updates, casting, and online metadata (credits, biographies and reviews from
MusicBrainz and Wikipedia), which is off until an administrator turns it on.
FETCH and the AI enhancer are your own machines on your network; nothing about
them goes to a cloud service.

[Privacy and security](https://parson.dev/docs/accounts-privacy-security)

## Development

Requires Bun, Rust, Node.js, FFmpeg, and the native build toolchain for your
platform.

```sh
bun install --frozen-lockfile
cargo run -p parson-music
```

Run the web app separately:

```sh
bun --filter parson-music-web dev
```

[Development guide](https://parson.dev/docs/development)

## Repository

- `crates/parson-core` — accounts, identities, and libraries
- `crates/backend` — server, indexer, storage, APIs, and discovery
- `apps/web` — web player
- `apps/desktop` — desktop packaging
- `apps/site` — website and docs

## Security

Report vulnerabilities privately through
[GitHub Security Advisories](https://github.com/mavingaw/parson-jin/security/advisories/new).

Deployment guidance is in the
[privacy and security documentation](https://parson.dev/docs/accounts-privacy-security).

## License

[GPL-3.0-only](LICENSE)

Mujin Music is based on [Parson](https://github.com/ParsonLabs/Parson) by
ParsonLabs and keeps its GPL-3.0-only licence and copyright notices. Third-party
components are listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
The Mujin Music fonts (Jost, Inter, JetBrains Mono) are under the SIL Open
Font License 1.1; each ships with its `OFL.txt` in `apps/web/public/fonts`.
