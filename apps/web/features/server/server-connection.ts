import { connectDesktopServer } from "@/lib/desktop/bridge";
import { toast } from "sonner";

export const OFFICIAL_PARSON_PORT = 1993;

export function normalizeServerOrigin(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return "";
  const candidate = /^[a-z][a-z\d+.-]*:\/\//i.test(trimmed)
    ? trimmed
    : `http://${trimmed}`;

  try {
    const url = new URL(candidate);
    if (url.protocol !== "http:" && url.protocol !== "https:") return "";
    if (url.username || url.password || !url.hostname) return "";
    if (url.protocol === "http:" && !url.port) {
      url.port = String(OFFICIAL_PARSON_PORT);
    }
    return url.origin;
  } catch {
    return "";
  }
}

export function serverConnectionTarget(origin: string, libraryName?: string) {
  const normalized = normalizeServerOrigin(origin);
  if (!normalized) return "";
  const destination = libraryName
    ? `/login?library=${encodeURIComponent(libraryName)}`
    : "/";
  return `${normalized}${destination}`;
}

export function connectToServer(origin: string, libraryName?: string) {
  const target = serverConnectionTarget(origin, libraryName);
  if (!target || typeof window === "undefined") return false;
  if (window.__PARSON_ELECTRON__) {
    // Parson Desktop switches itself to the other server (as a remote that
    // never starts its own), checking it is really Parson first.
    void connectDesktopServer(normalizeServerOrigin(origin)).catch(
      (error: unknown) => {
        toast(
          error instanceof Error
            ? error.message.replace(
                /^Error invoking remote method '[^']+': (Error: )?/,
                "",
              )
            : "Could not connect to that server.",
        );
      },
    );
  } else {
    // Navigating to the server keeps browser authentication same-origin, so
    // access and refresh credentials can remain in HttpOnly cookies.
    window.location.assign(target);
  }
  return true;
}
