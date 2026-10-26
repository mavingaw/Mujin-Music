"use client";

type DesktopPlatform = "linux" | "macos" | "windows";
export type DesktopUpdateState = {
  status:
    | "manual"
    | "idle"
    | "checking"
    | "current"
    | "downloading"
    | "ready"
    | "error";
  version: string;
  availableVersion?: string;
  percent?: number;
  message: string;
  releaseUrl: string;
};

/**
 * How Parson Desktop is set up: `local` runs the Parson server on this
 * computer; `remote` is a remote for a server elsewhere on the network.
 */
export type DesktopServerMode = "local" | "remote";
export type DesktopServerState = {
  mode: DesktopServerMode | null;
  name: string;
  origin: string;
  localAvailable: boolean;
  /**
   * True while this computer's own background server is running, even when
   * this window is a remote for another server. Older desktop builds omit it.
   */
  localRunning?: boolean;
};
export type DesktopServerEvent = { type: "restarted" };

declare global {
  interface Window {
    __PARSON_ELECTRON__?: {
      serverMode?: DesktopServerMode | null;
      invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
      windowControls: {
        close(): Promise<boolean>;
        isMaximized(): Promise<boolean>;
        minimize(): Promise<boolean>;
        toggleMaximize(): Promise<boolean>;
        watchMaximized(callback: (maximized: boolean) => void): void;
      };
      watchPowerResume(callback: () => void): () => void;
      watchUpdates(callback: (state: DesktopUpdateState) => void): () => void;
      watchServer?(callback: (event: DesktopServerEvent) => void): () => void;
    };
  }
}

export function watchDesktopUpdates(
  callback: (state: DesktopUpdateState) => void,
) {
  return window.__PARSON_ELECTRON__?.watchUpdates?.(callback) ?? (() => {});
}

export async function desktopUpdates(action: "state" | "check" | "install") {
  return electronInvoke<DesktopUpdateState>(`updates_${action}`);
}

const electronInvoke = <T>(command: string, args?: Record<string, unknown>) =>
  window.__PARSON_ELECTRON__?.invoke<T>(command, args) ?? null;

export function hasDesktopBridge(): boolean {
  return (
    typeof window !== "undefined" && Boolean(window.__PARSON_ELECTRON__?.invoke)
  );
}

export function electronWindowControls() {
  if (typeof window === "undefined") {
    return null;
  }
  return window.__PARSON_ELECTRON__?.windowControls ?? null;
}

export function watchDesktopPowerResume(callback: () => void) {
  if (typeof window === "undefined") {
    return () => {};
  }
  return window.__PARSON_ELECTRON__?.watchPowerResume(callback) ?? (() => {});
}

export async function desktopPlatform(): Promise<DesktopPlatform | null> {
  if (typeof window === "undefined") {
    return null;
  }
  return electronInvoke<DesktopPlatform>("platform");
}

export async function selectMusicFolder(): Promise<string | null> {
  if (typeof window === "undefined") {
    return null;
  }
  return electronInvoke<string | null>("select_music_folder");
}

export async function showTrackInFileManager(path: string): Promise<boolean> {
  if (typeof window === "undefined") {
    return false;
  }
  return (
    (await electronInvoke<boolean>("show_track_in_file_manager", { path })) ??
    false
  );
}

/**
 * True when this page runs inside Parson Desktop and the desktop app also
 * runs the server, so native helpers that touch this computer's files (the
 * folder picker, "show in folder") act on the same machine as the library.
 */
export function hasLocalDesktopBridge(): boolean {
  return (
    hasDesktopBridge() && window.__PARSON_ELECTRON__?.serverMode !== "remote"
  );
}

export function desktopServerMode(): DesktopServerMode | null {
  if (typeof window === "undefined") return null;
  return window.__PARSON_ELECTRON__?.serverMode ?? null;
}

export async function desktopServerState(): Promise<DesktopServerState | null> {
  if (typeof window === "undefined") return null;
  try {
    return (await electronInvoke<DesktopServerState>("server_state")) ?? null;
  } catch {
    return null;
  }
}

/** Opens Parson Desktop's server chooser. */
export async function chooseDesktopServer(): Promise<boolean> {
  if (typeof window === "undefined") return false;
  return Boolean(await electronInvoke<boolean>("server_choose"));
}

/** Makes Parson Desktop a remote for the server at `origin`. */
export async function connectDesktopServer(origin: string): Promise<boolean> {
  if (typeof window === "undefined") return false;
  return Boolean(await electronInvoke<boolean>("server_connect", { origin }));
}

/** Makes Parson Desktop run the server on this computer. */
export async function runDesktopLocalServer(): Promise<boolean> {
  if (typeof window === "undefined") return false;
  return Boolean(await electronInvoke<boolean>("server_use_local"));
}

export function watchDesktopServer(
  callback: (event: DesktopServerEvent) => void,
) {
  if (typeof window === "undefined") return () => {};
  return window.__PARSON_ELECTRON__?.watchServer?.(callback) ?? (() => {});
}

/**
 * The server's background service on this computer ("Use this computer"):
 * the Windows tray host, a macOS LaunchAgent with a menu-bar item, or a
 * systemd user service on Linux. It runs apart from the desktop window.
 */
export type DesktopServiceKind =
  "windows-tray" | "launch-agent" | "systemd-user" | "xdg-autostart";
export type DesktopServiceState = {
  kind: DesktopServiceKind | null;
  label?: string;
  available: boolean;
  installed?: boolean;
  running?: boolean;
  autostart?: boolean;
};

export async function desktopServiceState(): Promise<DesktopServiceState | null> {
  if (typeof window === "undefined") return null;
  try {
    return (
      (await electronInvoke<DesktopServiceState>("server_service")) ?? null
    );
  } catch {
    return null;
  }
}

export async function updateDesktopService(change: {
  autostart?: boolean;
  action?: "start" | "stop" | "restart";
}): Promise<DesktopServiceState | null> {
  if (typeof window === "undefined") return null;
  return (
    (await electronInvoke<DesktopServiceState>("server_service_set", change)) ??
    null
  );
}
