# Mujin Music Mobile

Expo client for Mujin Music servers on the local network.

## Development

```bash
bun install
bun run start
bun run ios
bun run web
bun run lint
bun run type-check
```

## Android builds

`android/` is generated and not committed. A release APK for one ABI:

```bash
cd apps/mobile
bunx expo prebuild --platform android --clean --no-install
cd android
./gradlew assembleRelease -PreactNativeArchitectures=arm64-v8a
```

Release builds need the upload key as Gradle properties (read by
`plugins/with-android-release-signing.js`): `PARSON_UPLOAD_STORE_FILE`,
`PARSON_UPLOAD_STORE_PASSWORD`, `PARSON_UPLOAD_KEY_ALIAS` and
`PARSON_UPLOAD_KEY_PASSWORD`, for example as `ORG_GRADLE_PROJECT_*`
environment variables. For a local test build, point them at the debug key
prebuild writes (`debug.keystore`, `android`, `androiddebugkey`, `android`).

CI (`.github/workflows/android.yml`) builds the arm64-v8a release APK on every
pull request and uploads it as `mujin-music-android-arm64-v8a`. It signs with
the upload key when the repository has the secrets
`ANDROID_UPLOAD_KEYSTORE_BASE64` (the keystore, base64-encoded),
`ANDROID_UPLOAD_STORE_PASSWORD`, `ANDROID_UPLOAD_KEY_ALIAS` and
`ANDROID_UPLOAD_KEY_PASSWORD`. Without them (forks, or before the key is set
up) it signs the same build with the debug key and names the artifact
`mujin-music-android-arm64-v8a-debug-signed`: it installs for testing, but
cannot update an install signed with the upload key.

The debug key is the one in Expo's project template
(`android/app/debug.keystore`, certificate SHA-256
`fac61745dc0903786fb9ede62a962b399f7348f0bb6f899b8332667591033b9c`), so it is
the same in every build and debug-signed APKs update each other. CI fails if
a debug-signed APK carries any other certificate, or if the upload key
secrets hold this public key.

The stable release (`.github/workflows/release.yml`) calls the same workflow
for `arm64-v8a` and `armeabi-v7a` and publishes
`Mujin-Music-<version>-android-<abi>.apk`, signed the same way; the release
notes say which key signed them. `versionName` is the release version and
`versionCode` is `major * 1000000 + minor * 1000 + patch`, both written to
`app.json` by `bun run version:sync`.

## Licensing

GPL-3.0-only. See the root and local `LICENSE` files.
