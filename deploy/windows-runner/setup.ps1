<#
.SYNOPSIS
    Installs the Gitea runner on this Windows PC as a Windows service that
    runs the Mujin Music Windows build jobs (label windows-x64, host mode).

.DESCRIPTION
    Run from an elevated PowerShell 7 window in a clone of the repository:

        pwsh -ExecutionPolicy Bypass -File deploy\windows-runner\setup.ps1

    The script
      1. checks the build prerequisites (Git, Node.js 24, Bun, rustup and the
         Rust toolchain from rust-toolchain.toml, the Visual Studio C++ build
         tools) and, with -InstallBuildTools, installs Visual Studio 2022
         Build Tools with the C++ workload through winget;
      2. adds the Rust target for the arm64 Windows build;
      3. downloads the Gitea runner and the WinSW service wrapper at pinned
         versions and checks their SHA-256 checksums;
      4. registers the runner with the Gitea server (the registration token is
         asked for and never written to disk by this script; the runner keeps
         its own credentials in .runner);
      5. installs and starts the service under your Windows account, so the
         jobs find the toolchains installed for you, with the right to log on
         as a service.

    Nothing secret is stored in this file. See README.md next to it.

.PARAMETER InstanceUrl
    The Gitea address the runner talks to (Gitea's ROOT_URL).

.PARAMETER RegistrationToken
    The runner registration token from Gitea (Site Administration ->
    Actions -> Runners -> Create new runner, or the repository's
    Settings -> Actions -> Runners). Asked for when not given.

.PARAMETER InstallBuildTools
    Install Visual Studio 2022 Build Tools with the C++ workload, the ARM64
    build tools, the Windows 11 SDK and Clang (needed by the arm64 build),
    through winget.

.PARAMETER Uninstall
    Stop and remove the service. The runner stays registered in Gitea until
    you delete it there.
#>
[CmdletBinding()]
param(
    [string]$InstanceUrl = "http://192.168.1.58:3002",
    [SecureString]$RegistrationToken,
    [string]$RunnerName = $env:COMPUTERNAME,
    [string]$Labels = "windows-x64:host",
    [string]$InstallDirectory = "C:\mujin-runner",
    [System.Management.Automation.PSCredential]$ServiceCredential,
    [switch]$InstallBuildTools,
    [switch]$AddDefenderExclusion,
    [switch]$SkipPrerequisiteCheck,
    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

# Pinned downloads. To update, change the version, download the file and
# put its SHA-256 here (Get-FileHash -Algorithm SHA256).
$RunnerVersion = "5.1.0"
$RunnerUrl = "https://gitea.com/gitea/runner/releases/download/v$RunnerVersion/gitea-runner-$RunnerVersion-windows-amd64.exe"
$RunnerSha256 = "8a732ddde63a93148b919df1d5a33b760bdbfd39ad8b6dc3f9f24bf44764e020"
$WinSwVersion = "2.12.0"
$WinSwUrl = "https://github.com/winsw/winsw/releases/download/v$WinSwVersion/WinSW-x64.exe"
$WinSwSha256 = "05b82d46ad331cc16bdc00de5c6332c1ef818df8ceefcd49c726553209b3a0da"
$RustToolchain = "1.99.0"
$ServiceId = "mujin-gitea-runner"

$BuildToolsComponents = @(
    "Microsoft.VisualStudio.Workload.VCTools",
    "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
    "Microsoft.VisualStudio.Component.VC.Tools.ARM64",
    "Microsoft.VisualStudio.Component.Windows11SDK.26100",
    "Microsoft.VisualStudio.Component.VC.Llvm.Clang"
)

function Assert-Administrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw "Run this script from an elevated PowerShell window (Run as administrator)."
    }
}

function Get-VsWhere {
    $path = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path -LiteralPath $path) { return $path }
    return $null
}

function Test-VisualStudioComponent([string]$Component) {
    $vswhere = Get-VsWhere
    if (-not $vswhere) { return $false }
    $found = & $vswhere -products * -requires $Component -property installationPath
    return [bool]$found
}

function Install-BuildTools {
    if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
        throw "winget is not available; install 'App Installer' from the Microsoft Store first."
    }
    $override = "--quiet --wait --norestart --nocache " +
        (($BuildToolsComponents | ForEach-Object { "--add $_" }) -join " ") +
        " --includeRecommended"
    Write-Host "Installing Visual Studio 2022 Build Tools (this takes a while)..."
    & winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --source winget `
        --accept-package-agreements --accept-source-agreements --override $override
    # 3010: installed, a restart is needed.
    if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne 3010) {
        throw "Installing the Visual Studio Build Tools failed (winget exit code $LASTEXITCODE). If they are already installed, add the missing components in the Visual Studio Installer instead."
    }
    if ($LASTEXITCODE -eq 3010) {
        Write-Warning "Restart Windows after this script finishes, before the first build."
    }
}

function Test-Prerequisites {
    $problems = @()
    foreach ($tool in @("git", "node", "bun", "rustup", "cargo", "pwsh")) {
        if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
            $problems += "$tool is not on PATH."
        }
    }
    if (Get-Command node -ErrorAction SilentlyContinue) {
        $major = [int]((& node --version).TrimStart("v").Split(".")[0])
        if ($major -lt 24) { $problems += "Node.js 24 or later is needed (found $(& node --version))." }
    }
    if (Get-Command rustup -ErrorAction SilentlyContinue) {
        $toolchains = & rustup toolchain list
        if (-not ($toolchains -match "^$([regex]::Escape($RustToolchain))")) {
            $problems += "Rust $RustToolchain is not installed (rustup toolchain install $RustToolchain)."
        }
    }
    foreach ($component in $BuildToolsComponents | Select-Object -Skip 1) {
        if (-not (Test-VisualStudioComponent $component)) {
            $problems += "Visual Studio component $component is missing (run again with -InstallBuildTools)."
        }
    }
    return $problems
}

function Get-VerifiedDownload([string]$Url, [string]$Sha256, [string]$Destination) {
    $partial = "$Destination.download"
    Invoke-WebRequest -Uri $Url -OutFile $partial -UseBasicParsing
    $actual = (Get-FileHash -LiteralPath $partial -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $Sha256) {
        Remove-Item -LiteralPath $partial -Force
        throw "Checksum mismatch for $Url (expected $Sha256, got $actual)."
    }
    Move-Item -LiteralPath $partial -Destination $Destination -Force
}

function Grant-ServiceLogonRight([string]$Account) {
    $ntAccount = New-Object System.Security.Principal.NTAccount($Account)
    $sid = $ntAccount.Translate([System.Security.Principal.SecurityIdentifier]).Value
    $temporary = Join-Path $env:TEMP "mujin-runner-rights"
    New-Item -ItemType Directory -Path $temporary -Force | Out-Null
    $exported = Join-Path $temporary "export.inf"
    & secedit /export /cfg $exported /areas USER_RIGHTS | Out-Null
    $line = Select-String -LiteralPath $exported -Pattern "^SeServiceLogonRight\s*=" | Select-Object -First 1
    $current = if ($line) { ($line.Line -split "=", 2)[1].Trim() } else { "" }
    if ($current -split "," | Where-Object { $_.Trim() -eq "*$sid" }) {
        Remove-Item -LiteralPath $temporary -Recurse -Force
        return
    }
    $value = if ($current) { "$current,*$sid" } else { "*$sid" }
    $policy = Join-Path $temporary "grant.inf"
    @(
        "[Unicode]",
        "Unicode=yes",
        "[Version]",
        'signature="$CHICAGO$"',
        "Revision=1",
        "[Privilege Rights]",
        "SeServiceLogonRight = $value"
    ) | Set-Content -LiteralPath $policy -Encoding Unicode
    & secedit /configure /db (Join-Path $temporary "grant.sdb") /cfg $policy /areas USER_RIGHTS | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Could not grant $Account the right to log on as a service." }
    Remove-Item -LiteralPath $temporary -Recurse -Force
}

function ConvertFrom-SecureText([SecureString]$Secure) {
    $pointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($Secure)
    try { return [Runtime.InteropServices.Marshal]::PtrToStringBSTR($pointer) }
    finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($pointer) }
}

Assert-Administrator
$wrapper = Join-Path $InstallDirectory "$ServiceId.exe"

if ($Uninstall) {
    if (Test-Path -LiteralPath $wrapper) {
        & $wrapper stop | Out-Host
        & $wrapper uninstall | Out-Host
    }
    Write-Host "Removed the $ServiceId service. Delete the runner in Gitea's runner list as well."
    return
}

if ($InstallBuildTools) { Install-BuildTools }

if (-not $SkipPrerequisiteCheck) {
    $problems = @(Test-Prerequisites)
    if ($problems.Count -gt 0) {
        $problems | ForEach-Object { Write-Warning $_ }
        throw "Install the missing prerequisites (see README.md), or pass -SkipPrerequisiteCheck."
    }
}

# The arm64 Windows packages are cross-built on this x64 PC.
& rustup target add aarch64-pc-windows-msvc --toolchain $RustToolchain
if ($LASTEXITCODE -ne 0) { throw "Adding the aarch64-pc-windows-msvc Rust target failed." }

New-Item -ItemType Directory -Path $InstallDirectory, (Join-Path $InstallDirectory "work"),
    (Join-Path $InstallDirectory "cache"), (Join-Path $InstallDirectory "logs") -Force | Out-Null
$runner = Join-Path $InstallDirectory "gitea-runner.exe"
if (Test-Path -LiteralPath (Join-Path $InstallDirectory ".runner")) {
    Write-Host "The runner is already registered; keeping its registration."
}

if (Test-Path -LiteralPath $wrapper) {
    & $wrapper stop 2>$null | Out-Null
}
Write-Host "Downloading the Gitea runner $RunnerVersion and WinSW $WinSwVersion..."
Get-VerifiedDownload $RunnerUrl $RunnerSha256 $runner
Get-VerifiedDownload $WinSwUrl $WinSwSha256 $wrapper

$config = Join-Path $InstallDirectory "config.yaml"
$work = (Join-Path $InstallDirectory "work") -replace "\\", "/"
$cache = (Join-Path $InstallDirectory "cache") -replace "\\", "/"
@"
# Gitea runner settings for the Windows build PC (written by setup.ps1).
log:
  level: info
runner:
  file: .runner
  # One Windows build at a time: the jobs install and uninstall the app.
  capacity: 1
  timeout: 5h
  labels:
    - "$Labels"
cache:
  enabled: true
  dir: "$cache"
host:
  # A short path keeps Electron and Cargo below Windows' path length limits.
  workdir_parent: "$work"
"@ | Set-Content -LiteralPath $config -Encoding utf8

if (-not (Test-Path -LiteralPath (Join-Path $InstallDirectory ".runner"))) {
    if (-not $RegistrationToken) {
        $RegistrationToken = Read-Host -AsSecureString "Gitea runner registration token"
    }
    $token = ConvertFrom-SecureText $RegistrationToken
    Push-Location $InstallDirectory
    try {
        & $runner --config $config register --no-interactive --instance $InstanceUrl `
            --token $token --name $RunnerName --labels $Labels
        if ($LASTEXITCODE -ne 0) { throw "Registering the runner with $InstanceUrl failed." }
    } finally {
        $token = $null
        Pop-Location
    }
}

@"
<service>
  <id>$ServiceId</id>
  <name>Mujin Music Gitea runner</name>
  <description>Runs the Mujin Music Windows build jobs for $InstanceUrl.</description>
  <executable>%BASE%\gitea-runner.exe</executable>
  <arguments>--config "%BASE%\config.yaml" daemon</arguments>
  <workingdirectory>%BASE%</workingdirectory>
  <startmode>Automatic</startmode>
  <delayedAutoStart>true</delayedAutoStart>
  <onfailure action="restart" delay="30 sec" />
  <logpath>%BASE%\logs</logpath>
  <log mode="roll-by-size">
    <sizeThreshold>10240</sizeThreshold>
    <keepFiles>8</keepFiles>
  </log>
</service>
"@ | Set-Content -LiteralPath (Join-Path $InstallDirectory "$ServiceId.xml") -Encoding utf8

$installed = Get-Service -Name $ServiceId -ErrorAction SilentlyContinue
if (-not $installed) {
    & $wrapper install | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "Installing the service failed." }
}

# The service runs as you: the jobs use your Node, Bun and Rust, and your
# administrator rights (the installer and firewall checks need them).
if (-not $ServiceCredential) {
    $ServiceCredential = Get-Credential -UserName "$env:USERDOMAIN\$env:USERNAME" `
        -Message "Windows account the runner service runs as (your sign-in password)"
}
Grant-ServiceLogonRight $ServiceCredential.UserName
$service = Get-CimInstance -ClassName Win32_Service -Filter "Name='$ServiceId'"
$result = Invoke-CimMethod -InputObject $service -MethodName Change -Arguments @{
    StartName = $ServiceCredential.UserName
    StartPassword = $ServiceCredential.GetNetworkCredential().Password
}
if ($result.ReturnValue -ne 0) {
    throw "Could not set the service account (Win32_Service.Change returned $($result.ReturnValue))."
}

if ($AddDefenderExclusion) {
    # Faster builds: Defender does not scan the build folders as they change.
    Add-MpPreference -ExclusionPath $InstallDirectory
}

Start-Service -Name $ServiceId
Get-Service -Name $ServiceId | Format-Table -AutoSize | Out-Host
Write-Host "The runner is running. It appears in Gitea under Site Administration -> Actions -> Runners."
