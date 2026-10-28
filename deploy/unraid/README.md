# Building Mujin Music on your own machines

This guide moves the Mujin Music code and its builds to the Gitea server
already running on the Unraid server (MujinServer, `192.168.1.58`). GitHub
stays as a public copy: Gitea pushes every change there, and GitHub's own
machines keep building the macOS app, which needs a Mac.

Nothing here installs a second Gitea. You add build runners next to the
Gitea you have, change a few Gitea settings, and set some secrets.

## What runs where

| What                                                                           | Where                                                | How                                                    |
| ------------------------------------------------------------------------------ | ---------------------------------------------------- | ------------------------------------------------------ |
| Code, issues, pull requests, releases                                          | Gitea on MujinServer (`http://192.168.1.58:3002`)    | your existing Gitea container                          |
| Public copy of the code and releases                                           | GitHub (`mavingaw/Mujin-Music`)                      | Gitea push mirror; the release job copies releases     |
| Rust checks and tests, web and site, Home Assistant and enhancer tests         | Linux runners                                        | `.gitea/workflows/test.yml`                            |
| Linux desktop (x64; arm64 optional, cross-built)                               | Linux runners                                        | `test.yml`                                             |
| Android APKs (64-bit and 32-bit)                                               | Linux runners                                        | `.gitea/workflows/android.yml`                         |
| Container image (amd64 and arm64), scanned with Trivy                          | Linux runners, pushed to Gitea's registry (and ghcr) | `.gitea/workflows/container-image.yml`                 |
| Windows desktop installer and portable app (x64 and arm64), Windows server zip | the Windows PC (MUJIN-DESK)                          | `test.yml`, Windows runner in host mode                |
| Windows firewall and installer checks                                          | the Windows PC                                       | `test.yml`                                             |
| macOS app (Intel and Apple silicon)                                            | GitHub-hosted Macs                                   | `.github/workflows/macos-for-gitea.yml`                |
| Copy of the macOS files into the Gitea release                                 | Linux runners                                        | `.gitea/workflows/macos-from-github.yml`, every 30 min |
| CodeQL security scanning                                                       | GitHub only                                          | `.github/workflows/codeql.yml` (unchanged)             |
| Install and upgrade tests of published releases                                | GitHub-hosted machines                               | `.github/workflows/published-artifacts.yml`            |

The Linux runners are the Unraid server and the front-desk Ubuntu box
(`192.168.1.103`); either can take any Linux job. The Windows PC takes the
Windows jobs, one at a time.

Gitea runs the workflows in `.gitea/workflows`. Because that folder exists,
Gitea ignores `.github/workflows`, which stay for GitHub. Until you set
`BUILDS_ON=gitea` on GitHub (step 10), GitHub keeps building everything as
before, so nothing breaks while you set this up.

## How releases and updates work

Gitea is only reachable on your home network, so phones and computers away
from home cannot download from it. For that reason a release is published
twice:

- on Gitea, for you;
- on GitHub, by the Gitea release job (with the `MIRROR_RELEASE_TOKEN`
  secret). The apps check GitHub for updates and the website links to
  GitHub downloads, wherever you are.

The macOS files are built on GitHub and added to the GitHub release about
an hour after the rest; until then, Mac users simply see no update. Gitea
copies them into its own release afterwards.

If you later make Gitea reachable from the internet over HTTPS, you can
switch the apps to it; see "Serving updates from Gitea" at the end.

## Before you start

- Gitea 1.27 or later (you have 1.27.2). Earlier versions run the shared
  workflows differently.
- An Unraid terminal (the `>_` icon) and the Windows PC.
- About 40 GB free on each Linux runner's Docker storage: the job image is
  about 12 GB, and Rust, Gradle and Docker caches grow to 20-30 GB. A Rust or
  Android job uses about 8 GB of memory.

## Step 1. Fix Gitea's settings

Gitea's settings file is
`/mnt/storage/appdata/gitea/gitea/conf/app.ini`. Stop the Gitea container
(Docker tab → gitea → Stop), open the file (for example with
`nano /mnt/storage/appdata/gitea/gitea/conf/app.ini` in the terminal), and
make these sections look like this (keep the other lines as they are):

```ini
[server]
; The address everyone (and the runners) uses to reach Gitea.
ROOT_URL = http://192.168.1.58:3002/
DOMAIN = 192.168.1.58
SSH_DOMAIN = 192.168.1.58
; The port people use for git over SSH (the container listens on 22).
SSH_PORT = 2222
SSH_LISTEN_PORT = 22
HTTP_PORT = 3000

[service]
; Nobody can create an account on your Gitea except you.
DISABLE_REGISTRATION = true

[actions]
ENABLED = true
; Long builds (the arm64 container image) need more than the 3h default.
ENDLESS_TASK_TIMEOUT = 5h

[packages]
ENABLED = true

[repository.release]
; Release files up to 2 GB (the default, stated for clarity).
FILE_MAX_SIZE = 2048
```

Start Gitea again. Check in Site Administration → Configuration that the
"Application URL" is `http://192.168.1.58:3002/`. If the Docker template
sets variables named `GITEA__server__ROOT_URL` or similar, they win over
`app.ini`: change or remove them in the template.

## Step 2. Bring the repository from GitHub into Gitea

1. On GitHub, create a token Gitea can read the repository with: Settings
   → Developer settings → Personal access tokens → Fine-grained tokens →
   Generate. Repository access: only `mavingaw/Mujin-Music`. Permissions:
   Contents, Issues, Pull requests and Metadata: read-only. Copy it.
2. In Gitea, click **+** (top right) → **New Migration** → **GitHub**.
3. URL `https://github.com/mavingaw/Mujin-Music`, paste the token, and tick
   Wiki, Labels, Milestones, Issues, Pull Requests and Releases. Owner:
   `mavingaw`, repository name `Mujin-Music`. Do **not** tick "This
   repository will be a mirror" (Gitea becomes the main copy).
4. Click **Migrate Repository**. Releases and their files come along.
5. On your own computers, point git at Gitea and keep GitHub as a second
   remote:

   ```sh
   git remote rename origin github
   git remote add origin http://192.168.1.58:3002/mavingaw/Mujin-Music.git
   git fetch origin
   git branch --set-upstream-to=origin/master master
   ```

## Step 3. Mirror Gitea to GitHub

1. On GitHub, create a second fine-grained token for the mirror: only
   `mavingaw/Mujin-Music`, permissions **Contents: Read and write** and
   **Workflows: Read and write** (without it GitHub refuses pushes that
   change `.github/workflows`). It is stored only in Gitea.
2. In Gitea: the repository → Settings → Repository → **Mirror Settings** →
   Push Mirror. Git Remote Repository URL
   `https://github.com/mavingaw/Mujin-Music.git`, username `mavingaw`,
   password: the token. Tick **Sync when commits are pushed**. Keep the
   interval (8h) as a safety net. Click **Add Push Mirror**, then
   **Synchronize Now** once.

From now on, push to Gitea only. The mirror overwrites GitHub with Gitea's
branches and tags, so anything committed directly on GitHub is lost.

## Step 4. Let Docker use Gitea's registry

The build image and the server image live in Gitea's package registry at
`192.168.1.58:3002`. That address has no HTTPS, so each machine that pulls
from it must be told to allow it.

- **Unraid**: Settings → Docker → set "Enable Docker" to No → Apply. In the
  terminal run

  ```sh
  echo 'DOCKER_OPTS="--insecure-registry=192.168.1.58:3002"' >> /boot/config/docker.cfg
  ```

  (if `docker.cfg` already has a `DOCKER_OPTS` line, add the option to it
  instead), then set "Enable Docker" back to Yes. Check with
  `docker info | grep -A2 "Insecure Registries"`.

- **Ubuntu box**: create `/etc/docker/daemon.json` with
  `{ "insecure-registries": ["192.168.1.58:3002"] }` (merge it into the file
  if it exists) and run `sudo systemctl restart docker`.

## Step 5. Create the tokens for builds

In Gitea, your avatar → Settings → Applications → **Generate New Token**:

1. Name `mujin-registry`, scope **package: Read and Write**. Copy it. This
   is `REGISTRY_TOKEN`.
2. Name `mujin-releases`, scope **repository: Read and Write**. Copy it.
   This is `RELEASE_TOKEN`.

## Step 6. Build the job image once

The Linux jobs run inside an image with all the build tools
(`deploy/runner-images/linux/Dockerfile`). Later versions are built by the
"Runner Image" workflow; the first one you build by hand. In the Unraid
terminal:

```sh
cd /mnt/storage/appdata
git clone http://192.168.1.58:3002/mavingaw/Mujin-Music.git mujin-src
cd mujin-src
docker build -t 192.168.1.58:3002/mavingaw/mujin-runner-linux:1 deploy/runner-images/linux
docker login 192.168.1.58:3002 -u mavingaw   # password: the REGISTRY_TOKEN
docker push 192.168.1.58:3002/mavingaw/mujin-runner-linux:1
```

This takes 15-30 minutes. The image's number is in
`deploy/runner-images/linux/VERSION`; when it changes, change the tag in
`config.yaml` (step 7) and restart the runners.

## Step 7. Start the Linux runners

In Gitea: Site Administration → Actions → Runners → **Create new runner**,
and copy the registration token (one token can register several runners).

**On Unraid** (with the Compose Manager plugin, or `docker compose` in the
terminal):

```sh
mkdir -p /mnt/storage/appdata/mujin-gitea-runner
cp -r /mnt/storage/appdata/mujin-src/deploy/unraid /mnt/storage/appdata/mujin-gitea-runner/compose
cd /mnt/storage/appdata/mujin-gitea-runner/compose
cp .env.example .env
nano .env        # paste the registration token, GITEA_RUNNER_NAME=mujinserver
docker compose up -d
docker logs -f mujin-gitea-runner   # wait for "Runner registered successfully"
```

Without Compose, the same container as one command:

```sh
docker run -d --name mujin-gitea-runner --restart unless-stopped --network host \
  -e CONFIG_FILE=/config/config.yaml \
  -e GITEA_INSTANCE_URL=http://192.168.1.58:3002 \
  -e GITEA_RUNNER_REGISTRATION_TOKEN=<token> \
  -e GITEA_RUNNER_NAME=mujinserver \
  -v /mnt/storage/appdata/mujin-gitea-runner/compose/config.yaml:/config/config.yaml:ro \
  -v /mnt/storage/appdata/mujin-gitea-runner:/data \
  -v /var/run/docker.sock:/var/run/docker.sock \
  docker.io/gitea/runner:5.1.0
```

**On the Ubuntu box**: install Docker, do step 4 there, copy the
`deploy/unraid` folder (for example to `/srv/mujin-gitea-runner/compose`),
set `GITEA_RUNNER_NAME=frontdesk` and `RUNNER_DATA=/srv/mujin-gitea-runner`
in `.env`, change `capacity: 3` to `capacity: 2` in `config.yaml`, and run
`docker compose up -d`.

After the first registration you can delete the token from `.env`. Both
runners appear under Site Administration → Actions → Runners with the label
`linux-x64`.

The runner gives every job this machine's Docker. Only push workflows you
trust, and keep account registration closed (step 1).

## Step 8. Set up the Windows runner

Follow [`deploy/windows-runner/README.md`](../windows-runner/README.md) on
the Windows PC. It ends with a runner labelled `windows-x64`.

## Step 9. Secrets and variables

In Gitea: the repository → Settings → Actions → **Secrets** and
**Variables**. (Gitea refuses names that start with `GITEA_` or `GITHUB_`,
which is why some names differ from GitHub's.)

| Secret                                                                                                                       | Needed for          | What                                                                                                                                                                                   |
| ---------------------------------------------------------------------------------------------------------------------------- | ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `REGISTRY_TOKEN`                                                                                                             | container image     | the `mujin-registry` token (step 5)                                                                                                                                                    |
| `RELEASE_TOKEN`                                                                                                              | releases            | the `mujin-releases` token (step 5)                                                                                                                                                    |
| `MIRROR_RELEASE_TOKEN`                                                                                                       | GitHub release copy | a GitHub fine-grained token for `mavingaw/Mujin-Music` with **Contents: Read and write**. Without it the release stays on Gitea only, and the apps (which check GitHub) see no update. |
| `ANDROID_UPLOAD_KEYSTORE_BASE64`, `ANDROID_UPLOAD_STORE_PASSWORD`, `ANDROID_UPLOAD_KEY_ALIAS`, `ANDROID_UPLOAD_KEY_PASSWORD` | optional            | the Android upload key, as on GitHub (all four or none; see `RELEASING.md`)                                                                                                            |
| `WINDOWS_CSC_LINK`, `WINDOWS_CSC_KEY_PASSWORD`                                                                               | optional            | Windows code signing                                                                                                                                                                   |
| `MINISIGN_SECRET_KEY`                                                                                                        | optional            | signs `SHA256SUMS` (see `RELEASING.md`)                                                                                                                                                |
| `GHCR_TOKEN`                                                                                                                 | optional            | a GitHub token with `write:packages` to also publish the image as `ghcr.io/mavingaw/mujin-music`                                                                                       |
| `PARSON_LIVE_*`                                                                                                              | optional            | the real-account streaming test (`live-streaming.yml`)                                                                                                                                 |

| Variable              | Default                                  | What                                                                                  |
| --------------------- | ---------------------------------------- | ------------------------------------------------------------------------------------- |
| `MUJIN_RELEASES_BASE` | GitHub (`…/Mujin-Music/releases`)        | where the apps look for updates; leave unset while Gitea is home-only                 |
| `MIRROR_REPOSITORY`   | `mavingaw/Mujin-Music`                   | the GitHub repository releases are copied to                                          |
| `CONTAINER_ARM64`     | on                                       | `false` builds the container for amd64 only (the arm64 build under emulation is slow) |
| `LINUX_ARM64`         | off                                      | `true` also cross-builds the arm64 Linux desktop app                                  |
| `CONTAINER_IMAGE`     | `192.168.1.58:3002/mavingaw/mujin-music` | the image the release notes tell people to pull (e.g. `ghcr.io/mavingaw/mujin-music`) |
| `REGISTRY_USER`       | the repository owner                     | the Gitea user the registry token belongs to                                          |

Then run **Actions → Build and Test → Run workflow** on `master` and check
that every job goes green.

## Step 10. Switch GitHub to macOS only

On GitHub: the repository → Settings → Secrets and variables → Actions →
Variables:

- `BUILDS_ON` = `gitea`. From now on GitHub skips the Linux, Windows,
  Android and container builds; macOS (`macos-for-gitea.yml`) and CodeQL
  keep running there.
- `MUJIN_RELEASES_BASE`: the same value as in Gitea, if you set one.
- Only if Gitea is reachable from the internet: `GITEA_URL` (its HTTPS
  address) and the secret `GITEA_RELEASE_TOKEN` (a Gitea token like
  `RELEASE_TOKEN`), so GitHub uploads the Mac files to Gitea directly. Not
  needed otherwise: Gitea fetches them.

Keep the macOS signing secrets (`MACOS_CSC_LINK` and friends) on GitHub.

To undo, delete `BUILDS_ON` on GitHub: GitHub builds everything again.

## Cutting a release

On your computer, with `origin` pointing at Gitea:

```sh
bun run release 1.2.0
git push origin HEAD --follow-tags
```

Or create the tag in Gitea's web UI (Releases → New Release, tag `v1.2.0`,
"Create tag only"). What happens next:

1. Gitea's **Publish Stable Release** builds everything except macOS (about
   1-2 hours; the arm64 container image is the longest part), publishes the
   release on Gitea, then on GitHub.
2. The push mirror brought the tag to GitHub within a minute; GitHub builds
   the macOS files meanwhile and adds them to the GitHub release once it
   exists, then tests the published release on every platform.
3. Within 30 minutes Gitea copies the macOS files into its own release.

To rebuild a release, run **Publish Stable Release** from Gitea's Actions
tab with the tag and "replace" ticked.

## Serving updates from Gitea (optional, later)

While Gitea is only on your home network, keep `MUJIN_RELEASES_BASE` unset:
the apps update from GitHub. If you later publish Gitea at an HTTPS address
(for example `https://git.mujindg.com` behind your Cloudflare):

- the apps need to download without signing in. Behind Cloudflare Access,
  add a **Bypass** policy (Everyone) limited to the paths
  `/mavingaw/Mujin-Music/releases/download/*` and
  `/api/v1/repos/mavingaw/Mujin-Music/releases*`, and `/v2/*` if people
  pull the container image from it. Leave the rest of Gitea behind Access.
- set `MUJIN_RELEASES_BASE=https://git.mujindg.com/mavingaw/Mujin-Music/releases`
  in Gitea and on GitHub. Builds made after that check Gitea for updates;
  existing installs move over with their next update from GitHub. Keep the
  GitHub copy (`MIRROR_RELEASE_TOKEN`) for older installs.
- keep `ROOT_URL` at the LAN address, or make the new name resolve to
  Unraid on your home network, so the runners keep reaching Gitea directly.
- Cloudflare limits uploads to 100 MB per request on the free plan, so
  GitHub cannot push the Mac files through it: leave `GITEA_URL` unset and
  let Gitea fetch them.

## Housekeeping

- Disk: `docker system prune` once a month on each Linux runner removes old
  job containers and images. The image builder keeps its cache in the
  `buildx_buildkit_mujin-gitea0` container; remove it to free space.
- Logs: `docker logs mujin-gitea-runner`.
- Runner updates: change the image tag in `docker-compose.yml` (pinned to
  `gitea/runner:5.1.0`) and run `docker compose up -d`.
