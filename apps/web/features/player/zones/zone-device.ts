import { createZoneDeviceId } from "@parson/music-sdk";

export const ZONE_DEVICE_STORAGE_KEY = "parson.zones.device-id";
export const SELECTED_ZONE_STORAGE_KEY = "parson.zones.selected";

type StorageLike = Pick<Storage, "getItem" | "setItem">;

function browserStorage(): StorageLike | null {
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    return null;
  }
}

/** A stable id for this browser or desktop install; random when storage fails. */
export function readOrCreateDeviceId(
  storage: StorageLike | null = browserStorage(),
  create: () => string = createZoneDeviceId,
): string {
  try {
    const stored = storage?.getItem(ZONE_DEVICE_STORAGE_KEY);
    if (stored && /^[a-z0-9_-]{8,64}$/i.test(stored)) return stored;
  } catch {}
  const id = create();
  try {
    storage?.setItem(ZONE_DEVICE_STORAGE_KEY, id);
  } catch {}
  return id;
}

export function readSelectedZone(
  storage: StorageLike | null = browserStorage(),
): string | null {
  try {
    return storage?.getItem(SELECTED_ZONE_STORAGE_KEY) || null;
  } catch {
    return null;
  }
}

export function storeSelectedZone(
  zoneId: string | null,
  storage:
    | (StorageLike & Pick<Storage, "removeItem">)
    | null = browserStorage() as Storage | null,
) {
  try {
    if (zoneId) storage?.setItem(SELECTED_ZONE_STORAGE_KEY, zoneId);
    else storage?.removeItem(SELECTED_ZONE_STORAGE_KEY);
  } catch {}
}

/** "Chrome on macOS", "Firefox on Windows", ... */
export function describeBrowser(userAgent: string): string {
  const browser = /Edg\//.test(userAgent)
    ? "Edge"
    : /OPR\//.test(userAgent)
      ? "Opera"
      : /Firefox\//.test(userAgent)
        ? "Firefox"
        : /Chrome\//.test(userAgent)
          ? "Chrome"
          : /Safari\//.test(userAgent)
            ? "Safari"
            : "Browser";
  const platform = /Windows/.test(userAgent)
    ? "Windows"
    : /iPhone|iPad/.test(userAgent)
      ? "iOS"
      : /Android/.test(userAgent)
        ? "Android"
        : /Mac OS X|Macintosh/.test(userAgent)
          ? "macOS"
          : /Linux|X11/.test(userAgent)
            ? "Linux"
            : "";
  return platform ? `${browser} on ${platform}` : browser;
}

export function clientZoneIdentity(
  userAgent: string,
  desktop: boolean,
): { kind: "web" | "desktop"; name: string; device: string } {
  const described = describeBrowser(userAgent);
  const platform = described.split(" on ")[1];
  if (desktop) {
    return {
      kind: "desktop",
      name: platform ? `Parson on ${platform}` : "Parson desktop",
      device: "Desktop app",
    };
  }
  return { kind: "web", name: described, device: "Web browser" };
}

/**
 * Where a play happened, for listening history: this device's zone id
 * (`client:<device id>`, as the server names it), its name and kind.
 */
export function localZoneForHistory(
  deviceId: string,
  userAgent: string,
  desktop: boolean,
): { zone_id?: string; zone_name: string; device: string } {
  const identity = clientZoneIdentity(userAgent, desktop);
  return {
    zone_id: /^[a-z0-9_-]{8,64}$/i.test(deviceId)
      ? `client:${deviceId}`
      : undefined,
    zone_name: identity.name,
    device: identity.kind === "desktop" ? identity.device : identity.name,
  };
}
