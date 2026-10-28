@echo off
rem Starts the Mujin Music enhancer on this Windows PC.
rem First start: installs packages, downloads the Apollo weights once and
rem prints the shared secret to paste into Mujin Music.
setlocal
cd /d "%~dp0"
title Mujin Music enhancer
where uv >nul 2>nul
if errorlevel 1 (
  echo uv is not installed. Install it once from https://docs.astral.sh/uv/ and run this again:
  echo   powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"
  pause
  exit /b 1
)
where ffmpeg >nul 2>nul
if errorlevel 1 (
  echo ffmpeg is not installed or not on PATH. Install it, for example:
  echo   winget install Gyan.FFmpeg
  pause
  exit /b 1
)
uv run --extra apollo mujin-enhancer serve %*
if errorlevel 1 pause
endlocal
