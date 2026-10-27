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

## Licensing

GPL-3.0-only. See the root and local `LICENSE` files.
