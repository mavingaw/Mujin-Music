# The Windows build runner

The Windows PC (MUJIN-DESK) builds the Windows installer and portable app
for x64 and arm64, the Windows server zip, and runs the Windows-only tests:
the WASAPI audio tests and the checks that the installer and the server's
scripts add and remove the right Windows Firewall rules.

It runs the Gitea runner as a Windows service in **host mode**: jobs run
directly on the PC, as your Windows account, with your Node, Bun and Rust.
`setup.ps1` installs it. The overall setup is in
[`deploy/unraid/README.md`](../unraid/README.md).

## What the PC needs

| Tool                                                                                                | Why                                                          | Install                                                                                  |
| --------------------------------------------------------------------------------------------------- | ------------------------------------------------------------ | ---------------------------------------------------------------------------------------- |
| Git                                                                                                 | checking out the code                                        | already installed                                                                        |
| Node.js 24                                                                                          | the build scripts, Electron                                  | already installed                                                                        |
| Bun                                                                                                 | JavaScript packages (the jobs use Bun 1.4.2 by themselves)   | already installed                                                                        |
| rustup and Rust 1.99.0 (`rust-toolchain.toml`)                                                      | the server and the bridge                                    | already installed; `setup.ps1` adds the `aarch64-pc-windows-msvc` target                 |
| PowerShell 7                                                                                        | the job scripts (`pwsh`)                                     | already installed                                                                        |
| Visual Studio 2022 Build Tools: C++ workload, MSVC x64 and ARM64 build tools, Windows 11 SDK, Clang | compiling Rust for Windows (Clang for the arm64 crypto code) | `setup.ps1 -InstallBuildTools` (winget package `Microsoft.VisualStudio.2022.BuildTools`) |

NSIS, which makes the installer, comes with electron-builder; nothing to
install.

The service runs as your account, which must be an administrator: the
firewall checks install the app silently and change firewall rules.

## Install

1. In Gitea: Site Administration → Actions → Runners → **Create new
   runner**. Copy the registration token.
2. Open **PowerShell 7 as administrator** (Start → type `pwsh` →
   right-click → Run as administrator) in your clone of the repository.
3. The first time, with the build tools:

   ```powershell
   pwsh -ExecutionPolicy Bypass -File deploy\windows-runner\setup.ps1 -InstallBuildTools -AddDefenderExclusion
   ```

   The Visual Studio installer takes 10-20 minutes. If it says a restart is
   needed, restart Windows and run the command again without
   `-InstallBuildTools`.

4. Paste the registration token when asked. Then Windows asks for the
   account the service runs as: keep your user name and type your Windows
   password (with a Microsoft account, its password, not the PIN).

The script downloads the Gitea runner 5.1.0 and the WinSW service wrapper
2.12.0, checks both against pinned SHA-256 checksums, registers the runner
as `MUJIN-DESK` with the label `windows-x64:host`, and starts the service
**Mujin Music Gitea runner**. Everything lives in `C:\mujin-runner`; the
jobs work in `C:\mujin-runner\work`. `-AddDefenderExclusion` stops
Microsoft Defender from scanning that folder, which makes builds much
faster.

Options: `-InstanceUrl` (default `http://192.168.1.58:3002`),
`-RunnerName`, `-InstallDirectory`, `-SkipPrerequisiteCheck`.

## Check it

- Gitea → Site Administration → Actions → Runners lists `MUJIN-DESK` as
  idle with `windows-x64`.
- `Get-Service mujin-gitea-runner` says Running.
- Logs: `C:\mujin-runner\logs`.

## Update, stop, remove

- Update the runner: change `$RunnerVersion` and `$RunnerSha256` at the top
  of `setup.ps1` and run it again; the registration is kept.
- Stop builds for a while: `Stop-Service mujin-gitea-runner`.
- Remove: `setup.ps1 -Uninstall`, then delete the runner in Gitea.
- After changing your Windows password, run `setup.ps1` again so the service
  gets the new one.
