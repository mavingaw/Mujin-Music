# Mujin Music server for Windows

Windows notification-area server with an embedded web UI.

## Use

Run `ParsonMusicServer.exe`. Data is stored in `%LOCALAPPDATA%\Parson` and the UI
opens at `http://127.0.0.1:1993`.

This is the server only, for a PC that serves music to the rest of the house.
Mujin Music Desktop also ships it: when you choose "Use this computer", the
desktop copies it to `%LOCALAPPDATA%\Programs\Parson Server`, starts it
with `--background` at sign-in, and connects to it as a remote, so closing
the desktop window never stops the server.
Use Mujin Music Desktop on other computers in remote mode ("Connect to a server on
my network"), the Android app, or any browser. The tray menu item **Connect a
phone or another computer…** shows this server's network address and a QR
code. Allow Mujin Music through Windows Firewall on private networks. See
[docs/server.md](../../docs/server.md).

Environment variables: `PARSON_DATA_DIR`, `PARSON_PORT`, and
`PARSON_LIBRARY_NAME`. FFmpeg is required for transcoding.

## Build and package

```powershell
bun install --frozen-lockfile
bun --filter parson-music-web build
.\crates\windows-server\package.ps1
```
