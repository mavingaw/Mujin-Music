# Releasing Mujin Music

The root `package.json` is the version source. The release command synchronises workspace packages, Rust manifests and lockfile, Expo version, iOS build number and Android versionCode, then creates a release commit and an annotated tag. It requires a clean checkout and a newer stable version.

```sh
bun run release 1.1.0
git push origin HEAD --follow-tags
```

Pushing the tag runs the stable release pipeline: `.github/workflows/release.yml` on GitHub, or, once the project builds on its own Gitea server, `.gitea/workflows/release.yml` there (see "Building on Gitea" below). It builds desktop installers for both architectures, the standalone Windows server, the Android APKs for `arm64-v8a` and `armeabi-v7a`, and versioned Docker images plus `latest`. The image is pushed only after every package has built. The release job checks the exact package set, generates desktop update channels and the server manifest from those exact artifacts, attests them, and writes the release body from `.github/release-notes.md` (rendered by `tools/release-notes.cjs`). No release-specific workflow edits are needed.

A v1.1.0 release publishes:

- `Mujin-Music-Setup-1.1.0-{x64,arm64}.exe`, `Mujin-Music-1.1.0-{x64,arm64}-portable.exe` and the installers' `.blockmap` files
- `Mujin-Music-1.1.0-{x64,arm64}.dmg`, `Mujin-Music-1.1.0-{x64,arm64}.zip` and their `.blockmap` files
- `Mujin-Music-1.1.0-{x86_64,arm64}.AppImage`, `Mujin-Music-1.1.0-{amd64,arm64}.deb`
- `Mujin-Music-Server-1.1.0-win-x64.zip`, `ParsonMusicServer.exe`, `ParsonMusicServer.exe.sha256`, `windows-server-update.json`
- `Mujin-Music-1.1.0-android-arm64-v8a.apk`, `Mujin-Music-1.1.0-android-armeabi-v7a.apk`
- `latest.yml`, `latest-mac.yml`, `latest-linux.yml`, `latest-linux-arm64.yml`
- the container image `ghcr.io/<owner>/<repository>:1.1.0` (lowercase), also tagged `latest`

Releases built on Gitea also publish `SHA256SUMS` (and `SHA256SUMS-macos` for the macOS files, which GitHub builds), plus a `.minisig` signature of each when a minisign key is configured, instead of GitHub build provenance for the non-macOS files. The container image is then `<gitea host>/<owner>/mujin-music`, and also `ghcr.io/…` when `GHCR_TOKEN` is set.

For an existing version, `bun run version:sync` refreshes generated version fields without committing or tagging. `bun run version:check` checks that everything agrees.

## One-time signing setup

For signed macOS desktop updates, configure `MACOS_CSC_LINK` and `MACOS_CSC_KEY_PASSWORD`, plus `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD`, and `APPLE_TEAM_ID` for notarization. Unsigned macOS builds still work but offer manual downloads instead of auto-updates. Windows signing optionally uses `WINDOWS_CSC_LINK` and `WINDOWS_CSC_KEY_PASSWORD`.

Android release APKs are signed with the upload key when all four of `ANDROID_UPLOAD_KEYSTORE_BASE64` (the keystore, base64-encoded), `ANDROID_UPLOAD_STORE_PASSWORD`, `ANDROID_UPLOAD_KEY_ALIAS` and `ANDROID_UPLOAD_KEY_PASSWORD` are set; with only some of them, the build fails. With none, the APKs are signed with the public debug key from Expo's project template, the same key CI uses, so they update each other in place, and the release notes say so. Android refuses to update an install signed with one key from an APK signed with another, so moving from the debug key to an upload key means uninstalling the app once. Set the upload key before the first release that people install if you can.

On GitHub, the container image is pushed with the workflow's `GITHUB_TOKEN`. A new GitHub package starts out private: make it public once (the package's settings on GitHub) so that `docker pull` works without signing in.

Signing secrets are inherited by the reusable build workflows. They are never stored in this repository.

## Building on Gitea

The project can host its code on a self-hosted Gitea and build there: `deploy/unraid/README.md` sets up the server, the Linux runners (Docker) and the Windows runner (host mode, `deploy/windows-runner`). The workflows in `.gitea/workflows` mirror those in `.github/workflows`; Gitea ignores `.github/workflows` once `.gitea/workflows` exists.

- Setting the GitHub repository variable `BUILDS_ON` to `gitea` makes the GitHub copies of the Linux, Windows, Android and container jobs skip, so nothing is built twice. macOS (`.github/workflows/macos-for-gitea.yml`) and CodeQL keep running on GitHub. Unset, GitHub builds everything as before.
- The Gitea release job publishes the release on Gitea and, with the `MIRROR_RELEASE_TOKEN` secret, the same files on the GitHub mirror. macOS packages follow: GitHub builds them on the mirrored tag and adds them to the GitHub release (and to the Gitea release directly when `GITEA_URL` is reachable from GitHub); `.gitea/workflows/macos-from-github.yml` otherwise copies them into the Gitea release. Until they arrive, the release carries the previous `latest-mac.yml` (and `latest-linux-arm64.yml` when the arm64 Linux build is off) with absolute links to the previous files (`tools/carry-update-feed.cjs`), so clients see no update rather than an error.
- `tools/publish-release.cjs` creates and fills releases through the Gitea and GitHub APIs. A new release is a draft until every file is uploaded.
- Signing with minisign: create a key without a password (`minisign -G -W -p mujin-music.pub -s mujin-music.key`), store the whole secret key file as the `MINISIGN_SECRET_KEY` secret (on Gitea, and on GitHub for the macOS checksums), and publish `mujin-music.pub` where users can find it. `minisign -Vm SHA256SUMS -p mujin-music.pub` checks a download list.

## Where updates come from

One build-time setting, `MUJIN_RELEASES_BASE`, names the releases page that packages download updates from, for example `https://git.example.com/mavingaw/Mujin-Music/releases`. Unset, it is the GitHub repository's (`https://github.com/mavingaw/Mujin-Music/releases`). It must use HTTPS. `tools/releases-base.cjs` derives the URLs: tagged files under `<base>/download/<tag>/`, the newest stable release under `<base>/latest/download/` on GitHub and `<base>/download/latest/` on Gitea or Forgejo. The setting is read by

- the desktop build (`apps/desktop/scripts/build.cjs`): electron-updater's generic provider reads `latest*.yml` from the newest release's folder, and Settings links to `<base>/latest`;
- the standalone Windows server (`crates/windows-server`): `src/updater.rs` reads it when compiled, and accepts only downloads under `<base>/download/<tag>/`; `package.ps1` writes the same URL into `windows-server-update.json`;
- the website's download links (`apps/site/lib/release-downloads.ts`);
- the release notes (`tools/release-notes.cjs`).

Set it as a repository variable (Gitea and GitHub alike) so every package of a release agrees. Builds made before this setting check GitHub; publishing each release on the GitHub mirror as well keeps them updating, and they move to the configured base with their next update.

## Update behaviour

Installed Windows and Linux AppImage releases check the stable release channel (`MUJIN_RELEASES_BASE`, GitHub by default) after startup and every six hours. Signed macOS releases use the same channel. Updates download without stopping playback and install on normal quit. Settings → Account provides **Check for updates** and **Restart to update**. The desktop backend is stopped before installation, and the server and embedded UI update together.

Development builds, portable Windows executables, unsigned macOS builds and Linux package installations offer manual release downloads or package-manager updates. Stable clients never install nightly releases or downgrade. Existing builds without updater integration require one manual installation of an updater-enabled build.

The standalone Windows server keeps its existing on-demand **Check and install updates** tray action. Its release manifest is now generated and published by the same pipeline.

Docker publishes new images; the container host controls replacement. To update the supplied Compose deployment:

```sh
docker compose pull
docker compose up -d
```

A host-managed updater can schedule those operations. The server does not replace its own container. The hosted web UI updates with its server; reload browser clients after deployment. Website deployment continues through the existing hosting setup, with download URLs derived from the package version.

Data directories and music stay separate from installers. Back up the data directory before an update that changes database schemas; executable rollback does not roll back a migrated database.

## Verification

```sh
bun run version:check
node --test tools/release.test.cjs
bun --filter parson-music-desktop desktop:test
node --test tools/*.test.cjs
```

Before public rollout, exercise installation from the previous signed release on each target platform. Local unit tests do not validate platform signing or a published feed.
