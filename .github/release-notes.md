<!--
The body of each stable GitHub release. release.yml renders it with
tools/release-notes.cjs, which replaces {{version}}, {{repository}}
(owner/name), {{image}} (the lowercase ghcr.io image) and {{certificate}}
(the SHA-256 of the Android signing certificate), keeps only the
android-signing block for the key that signed the APKs, and removes these
comments. The download names must match the files release.yml publishes;
tools/release.test.cjs checks that.
-->

# Mujin Music {{version}}

Mujin Music is a music server you run yourself, with apps for Windows,
macOS, Linux and Android. Put the server (or the desktop app, which
includes it) on the computer that holds your music, then play from your
other computers, phones and speakers on the same network.

## Downloads

| What                                                                     | Download                                                                                                                                                                                                                                                       |
| ------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Windows app** (includes the server)                                    | [Installer, 64-bit PC](https://github.com/{{repository}}/releases/download/v{{version}}/Mujin-Music-Setup-{{version}}-x64.exe) · [Installer, ARM PC](https://github.com/{{repository}}/releases/download/v{{version}}/Mujin-Music-Setup-{{version}}-arm64.exe) |
| Windows app, portable (no install)                                       | [64-bit PC](https://github.com/{{repository}}/releases/download/v{{version}}/Mujin-Music-{{version}}-x64-portable.exe) · [ARM PC](https://github.com/{{repository}}/releases/download/v{{version}}/Mujin-Music-{{version}}-arm64-portable.exe)                 |
| **Windows server** (no app window, tray only)                            | [Mujin-Music-Server-{{version}}-win-x64.zip](https://github.com/{{repository}}/releases/download/v{{version}}/Mujin-Music-Server-{{version}}-win-x64.zip) (with the Windows Firewall scripts)                                                                  |
| **macOS** (includes the server)                                          | [Apple silicon](https://github.com/{{repository}}/releases/download/v{{version}}/Mujin-Music-{{version}}-arm64.dmg) · [Intel](https://github.com/{{repository}}/releases/download/v{{version}}/Mujin-Music-{{version}}-x64.dmg)                                |
| **Linux** (includes the server)                                          | x86_64: [AppImage](https://github.com/{{repository}}/releases/download/v{{version}}/Mujin-Music-{{version}}-x86_64.AppImage) · [.deb](https://github.com/{{repository}}/releases/download/v{{version}}/Mujin-Music-{{version}}-amd64.deb)                      |
|                                                                          | ARM64: [AppImage](https://github.com/{{repository}}/releases/download/v{{version}}/Mujin-Music-{{version}}-arm64.AppImage) · [.deb](https://github.com/{{repository}}/releases/download/v{{version}}/Mujin-Music-{{version}}-arm64.deb)                        |
| **Android, 64-bit** (most phones and tablets)                            | [Mujin-Music-{{version}}-android-arm64-v8a.apk](https://github.com/{{repository}}/releases/download/v{{version}}/Mujin-Music-{{version}}-android-arm64-v8a.apk)                                                                                                |
| **Android, 32-bit** (older phones, tablets and smart displays)           | [Mujin-Music-{{version}}-android-armeabi-v7a.apk](https://github.com/{{repository}}/releases/download/v{{version}}/Mujin-Music-{{version}}-android-armeabi-v7a.apk)                                                                                            |
| **Docker** (server for Linux, a NAS or a Raspberry Pi; x86_64 and arm64) | `docker pull {{image}}:{{version}}` (also tagged `latest`)                                                                                                                                                                                                     |

For a server without a screen on Linux, use Docker; the
[server guide](https://github.com/{{repository}}/blob/v{{version}}/docs/server.md)
also covers systemd. On macOS and Linux the desktop app runs the server
itself.

## First run

### Windows

1. **SmartScreen.** The Windows downloads are not code-signed yet, so
   Windows may say "Windows protected your PC". Click **More info**, then
   **Run anyway**.
2. **Firewall.** Allow Mujin Music on **Private networks** when Windows asks
   (the installer adds this rule for you). With the server-only zip, unzip
   it to a folder you keep, then double-click
   **Allow through Windows Firewall.cmd** before starting
   `ParsonMusicServer.exe`. Make sure your home network is set to
   **Private** (Settings → Network & internet → your network → Network
   profile type), or phones and speakers cannot reach the server.
3. **FFmpeg.** Mujin Music uses FFmpeg to convert music that a phone or
   speaker cannot play as it is. It is not bundled yet; install it once
   from a terminal with `winget install Gyan.FFmpeg`, then restart Mujin
   Music.

### macOS and Linux

- macOS: open the `.dmg` and drag Mujin Music to Applications. The macOS
  downloads are not signed by Apple yet. If macOS says it cannot check the
  app, go to System Settings → Privacy & Security and click **Open
  Anyway**. If it says the app "is damaged", run
  `xattr -dr com.apple.quarantine "/Applications/Mujin Music.app"` in
  Terminal, then open it again.
- Linux: make the AppImage executable (`chmod +x Mujin-Music-*.AppImage`)
  and run it, or install the `.deb` with
  `sudo apt install ./Mujin-Music-{{version}}-amd64.deb` (ARM64:
  `-arm64.deb`). Install `ffmpeg` from your distribution for conversion.

### Android

1. Download the APK on the device: the 64-bit one for most phones and
   tablets, the 32-bit one for older devices (if the 64-bit APK says "App
   not installed" or "not compatible", use the 32-bit one).
2. Open it. Android asks to allow **Install unknown apps** for your browser
   or file manager: allow it, go back, and tap **Install**.
3. Open Mujin Music. It finds servers on your network by itself; if it does
   not, enter the server's address, such as `http://192.168.1.20:1993`.

<!-- android-signing: debug -->

**About the Android signature.** This project has no private Android
signing key yet, so these APKs are signed with the standard, publicly known
Android debug key (certificate SHA-256 `{{certificate}}`). Later releases
signed the same way install over this one as normal updates. Because that
key is public, Android cannot tell these APKs apart from someone else's
APK signed with the same key: only install Mujin Music APKs downloaded from
this page. When the project switches to its own key, you will need to
uninstall the app once before installing the new version; your server,
library and playlists are not affected, you only sign in again.

<!-- end android-signing -->
<!-- android-signing: upload -->

The Android APKs are signed with the Mujin Music release key (certificate
SHA-256 `{{certificate}}`).

<!-- end android-signing -->

### Docker

```sh
docker run -d --name mujin-music --network host \
  -v /path/to/mujin-data:/Parson -v /path/to/music:/music:ro \
  --restart unless-stopped {{image}}:{{version}}
```

Then open `http://<server-address>:1993`, create the administrator account
and choose `/music` as the music folder.

## Checking a download

Every file here was built by this repository's GitHub Actions from the
tagged source, with build provenance you can check:
`gh attestation verify <file> --repo {{repository}}`.

[All changes in this release](https://github.com/{{repository}}/commits/v{{version}})

---

Mujin Music is based on [Parson](https://github.com/ParsonLabs/parson) by
ParsonLabs. It is free software under the GNU GPL v3.
