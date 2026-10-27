; Mujin Music: inbound Windows Firewall rules (see docs/server.md,
; "Windows Firewall").
;
; Phones, browsers, other computers, Sonos and other UPnP speakers,
; Chromecasts and bridges reach the server over HTTP (TCP 1993 by default;
; speakers fetch the music from it), and the server finds speakers with SSDP
; and advertises itself with mDNS (UDP). On Windows the server is the tray
; host that the desktop app copies to
;   %LOCALAPPDATA%\Programs\Mujin Music Server\ParsonMusicServer.exe
; when "Use this computer" is chosen (electron/server-service.cjs), and the
; desktop window itself browses mDNS for servers. Without a rule, Windows
; asks once per program, and blocks inbound traffic without asking when the
; program starts in the background or the user is not an administrator.
;
; The installer adds one inbound allow rule per program, for private
; networks only; the uninstaller removes them. Rules are matched by name
; and program path, so another account's install keeps its own rules.
; Changing firewall rules needs administrator rights: an elevated install
; (for all users) adds them directly; a per-user install asks once through
; UAC; a silent install without administrator rights (an automatic update,
; a scripted install) never prompts and skips them. Updates keep the rules
; that are already there.

!include "LogicLib.nsh"

!define MUJIN_FIREWALL_APP_RULE "Mujin Music"
!define MUJIN_FIREWALL_SERVER_RULE "Mujin Music Server"
!define MUJIN_FIREWALL_SERVER_PROGRAM "Programs\Mujin Music Server\ParsonMusicServer.exe"
!define MUJIN_FIREWALL_SCRIPT "$PLUGINSDIR\mujin-music-firewall.cmd"
!define MUJIN_NETSH "%NETSH%"

; The server's path in the installing account's local application data. The
; environment is used rather than $LOCALAPPDATA, which points at ProgramData
; in an install for all users.
!macro mujinFirewallServerPath OUT
  ReadEnvStr ${OUT} LOCALAPPDATA
  StrCpy ${OUT} "${OUT}\${MUJIN_FIREWALL_SERVER_PROGRAM}"
!macroend

!macro mujinFirewallOpenScript
  InitPluginsDir
  FileOpen $R8 "${MUJIN_FIREWALL_SCRIPT}" w
  FileWrite $R8 `@echo off$\r$\n`
  ; The installer is 32-bit: prefer the native netsh over the WOW64 one.
  FileWrite $R8 `set "NETSH=%SystemRoot%\System32\netsh.exe"$\r$\n`
  FileWrite $R8 `if exist "%SystemRoot%\Sysnative\netsh.exe" set "NETSH=%SystemRoot%\Sysnative\netsh.exe"$\r$\n`
!macroend

!macro mujinFirewallDelete NAME PROGRAM
  FileWrite $R8 `"${MUJIN_NETSH}" advfirewall firewall delete rule name="${NAME}" dir=in program="${PROGRAM}" >nul 2>&1$\r$\n`
!macroend

!macro mujinFirewallAllow NAME PROGRAM DESCRIPTION
  !insertmacro mujinFirewallDelete "${NAME}" "${PROGRAM}"
  FileWrite $R8 `"${MUJIN_NETSH}" advfirewall firewall add rule name="${NAME}" dir=in action=allow program="${PROGRAM}" enable=yes profile=private description="${DESCRIPTION}"$\r$\n`
!macroend

; Runs the script with administrator rights: directly when the installer
; already has them, otherwise through one UAC prompt, and never prompts when
; silent.
!macro mujinFirewallRunScript
  FileClose $R8
  ${if} ${UAC_IsAdmin}
    nsExec::Exec `"$SYSDIR\cmd.exe" /c ""${MUJIN_FIREWALL_SCRIPT}""`
    Pop $R8
    DetailPrint "Windows Firewall rules for Mujin Music updated ($R8)."
  ${elseIfNot} ${Silent}
    ExecShellWait "runas" "${MUJIN_FIREWALL_SCRIPT}" "" SW_HIDE
    DetailPrint "Windows Firewall rules for Mujin Music requested."
  ${else}
    DetailPrint "Windows Firewall rules for Mujin Music skipped: a silent install without administrator rights cannot change them."
  ${endIf}
  Delete "${MUJIN_FIREWALL_SCRIPT}"
!macroend

!macro customInstall
  ; electron-builder's isUpdated test overwrites $R9, so these macros keep
  ; to $R6-$R8.
  Push $R6
  Push $R7
  Push $R8
  StrCpy $R6 "add"
  ${if} ${isUpdated}
    ; An update keeps the rules it already has (the paths do not change).
    nsExec::Exec `"$SYSDIR\netsh.exe" advfirewall firewall show rule name="${MUJIN_FIREWALL_APP_RULE}"`
    Pop $R8
    ${if} $R8 == "0"
      StrCpy $R6 "keep"
    ${endIf}
  ${endIf}
  ${if} $R6 == "add"
    !insertmacro mujinFirewallServerPath $R7
    !insertmacro mujinFirewallOpenScript
    !insertmacro mujinFirewallAllow "${MUJIN_FIREWALL_APP_RULE}" "$INSTDIR\${APP_EXECUTABLE_FILENAME}" "Lets Mujin Music find music servers on private networks (mDNS)."
    !insertmacro mujinFirewallAllow "${MUJIN_FIREWALL_SERVER_RULE}" "$R7" "Lets phones, browsers, speakers (Sonos, UPnP, Chromecast) and other computers reach the Mujin Music server on private networks."
    !insertmacro mujinFirewallRunScript
  ${endIf}
  Pop $R8
  Pop $R7
  Pop $R6
!macroend

!macro customUnInstall
  ${ifNot} ${isUpdated}
    Push $R7
    Push $R8
    !insertmacro mujinFirewallServerPath $R7
    !insertmacro mujinFirewallOpenScript
    !insertmacro mujinFirewallDelete "${MUJIN_FIREWALL_APP_RULE}" "$INSTDIR\${APP_EXECUTABLE_FILENAME}"
    ; The background server outlives the desktop app; its rule goes with it.
    ${ifNot} ${FileExists} "$R7"
      !insertmacro mujinFirewallDelete "${MUJIN_FIREWALL_SERVER_RULE}" "$R7"
    ${endIf}
    !insertmacro mujinFirewallRunScript
    Pop $R8
    Pop $R7
  ${endIf}
!macroend
