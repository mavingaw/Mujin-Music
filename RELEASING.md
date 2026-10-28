# Releasing Mujin Music

The root `package.json` is the version source. The release command synchronises workspace packages, Rust manifests and lockfile, Expo version, iOS build number and Android versionCode, then creates a release commit and an annotated tag. It requires a clean checkout and a newer stable version.

```sh
bun run release 1.1.0
git push origin HEAD --follow-tags
```

Pushing the tag runs the stable release pipeline (`.github/workflows/release.yml`). It builds desktop installers for both architectures, the standalone Windows server, the Android APKs for `arm64-v8a` and `armeabi-v7a`, and versioned Docker images plus `latest`. The image is pushed only after every package has built. The release job checks the exact package set, generates desktop update channels and the server manifest from those exact artifacts, attests them, and writes the release body from `.github/release-notes.md` (rendered by `tools/release-notes.cjs`). No release-specific workflow edits are needed.

A v1.1.0 release publishes:

- `Mujin-Music-Setup-1.1.0-{x64,arm64}.exe`, `Mujin-Music-1.1.0-{x64,arm64}-portable.exe` and the installers' `.blockmap` files
- `Mujin-Music-1.1.0-{x64,arm64}.dmg`, `Mujin-Music-1.1.0-{x64,arm64}.zip` and their `.blockmap` files
- `Mujin-Music-1.1.0-{x86_64,arm64}.AppImage`, `Mujin-Music-1.1.0-{amd64,arm64}.deb`
- `Mujin-Music-Server-1.1.0-win-x64.zip`, `ParsonMusicServer.exe`, `ParsonMusicServer.exe.sha256`, `windows-server-update.json`
- `Mujin-Music-1.1.0-android-arm64-v8a.apk`, `Mujin-Music-1.1.0-android-armeabi-v7a.apk`
- `latest.yml`, `latest-mac.yml`, `latest-linux.yml`, `latest-linux-arm64.yml`
- the container image `ghcr.io/<owner>/<repository>:1.1.0` (lowercase), also tagged `latest`

For an existing version, `bun run version:sync` refreshes generated version fields without committing or tagging. `bun run version:check` checks that everything agrees.

## One-time signing setup

For signed macOS desktop updates, configure `MACOS_CSC_LINK` and `MACOS_CSC_KEY_PASSWORD`, plus `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD`, and `APPLE_TEAM_ID` for notarization. Unsigned macOS builds still work but offer manual downloads instead of auto-updates. Windows signing optionally uses `WINDOWS_CSC_LINK` and `WINDOWS_CSC_KEY_PASSWORD`.

Android release APKs are signed with the upload key when all four of `ANDROID_UPLOAD_KEYSTORE_BASE64` (the keystore, base64-encoded), `ANDROID_UPLOAD_STORE_PASSWORD`, `ANDROID_UPLOAD_KEY_ALIAS` and `ANDROID_UPLOAD_KEY_PASSWORD` are set; with only some of them, the build fails. With none, the APKs are signed with the public debug key from Expo's project template, the same key CI uses, so they update each other in place, and the release notes say so. Android refuses to update an install signed with one key from an APK signed with another, so moving from the debug key to an upload key means uninstalling the app once. Set the upload key before the first release that people install if you can.

The container image is pushed with the workflow's `GITHUB_TOKEN`. A new GitHub package starts out private: make it public once (the package's settings on GitHub) so that `docker pull` works without signing in.

Signing secrets are inherited by the reusable build workflows. They are never stored in this repository.

## Update behaviour

Installed Windows and Linux AppImage releases check the stable GitHub channel after startup and every six hours. Signed macOS releases use the same channel. Updates download without stopping playback and install on normal quit. Settings → Account provides **Check for updates** and **Restart to update**. The desktop backend is stopped before installation, and the server and embedded UI update together.

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
```

Before public rollout, exercise installation from the previous signed release on each target platform. Local unit tests do not validate platform signing or a published feed.
