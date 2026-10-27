<#
.SYNOPSIS
Allows Mujin Music Server through Windows Firewall on private networks, or
removes that rule again.

.DESCRIPTION
Phones, browsers, other computers, Sonos and other UPnP speakers,
Chromecasts and bridges reach the server over HTTP (TCP 1993 by default;
speakers fetch the music from it), and the server finds speakers with SSDP
and advertises itself with mDNS (UDP). This adds one inbound allow rule,
named "Mujin Music Server", for ParsonMusicServer.exe on private networks
only. Rules are matched by name and program path, so other copies keep
theirs. Changing firewall rules needs administrator rights; the script asks
through UAC when it does not have them.

"Allow through Windows Firewall.cmd" and "Remove from Windows Firewall.cmd"
run this script with -Pause, so it can be started by double-clicking.

.PARAMETER Action
Allow (the default) adds the rule; Remove deletes it.

.PARAMETER Program
The server executable. Defaults to ParsonMusicServer.exe next to this script.
#>
param(
    [ValidateSet("Allow", "Remove")]
    [string]$Action = "Allow",
    [string]$Program = "",
    [switch]$Pause
)

$ErrorActionPreference = "Stop"
$ruleName = "Mujin Music Server"
$description = "Lets phones, browsers, speakers (Sonos, UPnP, Chromecast) and other computers reach the Mujin Music server on private networks."

function Wait-ForReader {
    if ($Pause -and -not [Console]::IsInputRedirected) {
        Read-Host "Press Enter to close" | Out-Null
    }
}

if (-not $Program) {
    $Program = Join-Path $PSScriptRoot "ParsonMusicServer.exe"
}
$Program = [IO.Path]::GetFullPath($Program)

$principal = New-Object Security.Principal.WindowsPrincipal(
    [Security.Principal.WindowsIdentity]::GetCurrent())
$elevated = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

if (-not $elevated) {
    Write-Output "Windows asks for permission to change the firewall rule."
    $arguments = @(
        "-NoProfile",
        "-ExecutionPolicy", "Bypass",
        "-File", "`"$PSCommandPath`"",
        "-Action", $Action,
        "-Program", "`"$Program`""
    )
    try {
        $process = Start-Process -FilePath "powershell.exe" -ArgumentList $arguments `
            -Verb RunAs -WindowStyle Hidden -Wait -PassThru
    } catch {
        Write-Output "The firewall rule was not changed: permission was not given."
        Wait-ForReader
        exit 1
    }
    if ($process.ExitCode -eq 0) {
        if ($Action -eq "Allow") {
            Write-Output "Mujin Music Server ($Program) is allowed through Windows Firewall on private networks."
        } else {
            Write-Output "The Windows Firewall rule for Mujin Music Server ($Program) was removed."
        }
    } else {
        Write-Output "Changing the Windows Firewall rule failed (exit code $($process.ExitCode))."
    }
    Wait-ForReader
    exit $process.ExitCode
}

# Remove this copy's rule first, so running the script again never
# duplicates it. netsh reports an error when there is none; that is fine.
& netsh.exe advfirewall firewall delete rule "name=$ruleName" dir=in "program=$Program" | Out-Null
$global:LASTEXITCODE = 0

if ($Action -eq "Allow") {
    & netsh.exe advfirewall firewall add rule "name=$ruleName" dir=in action=allow `
        "program=$Program" enable=yes profile=private "description=$description" | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Output "netsh could not add the Windows Firewall rule (exit code $LASTEXITCODE)."
        Wait-ForReader
        exit $LASTEXITCODE
    }
    Write-Output "Mujin Music Server ($Program) is allowed through Windows Firewall on private networks."
} else {
    Write-Output "The Windows Firewall rule for Mujin Music Server ($Program) was removed."
}
Wait-ForReader
exit 0
