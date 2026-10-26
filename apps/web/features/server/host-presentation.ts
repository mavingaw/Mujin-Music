import { hostOrigins, type HostInfo } from "@parson/music-sdk";

/** True for addresses only this computer can open (127.0.0.1, localhost). */
export function isLoopbackOrigin(origin: string): boolean {
  try {
    const host = new URL(origin).hostname;
    return (
      host === "localhost" ||
      host === "[::1]" ||
      /^127\.\d+\.\d+\.\d+$/.test(host)
    );
  } catch {
    return false;
  }
}

/**
 * The address to put in the QR code. If this page was itself opened through
 * a network address, that one is known to work, so it wins; otherwise the
 * server's best-ranked address.
 */
export function primaryHostOrigin(info: HostInfo, pageOrigin: string) {
  if (pageOrigin && !isLoopbackOrigin(pageOrigin)) return pageOrigin;
  return hostOrigins(info)[0] ?? "";
}

export type HostReachability =
  | { kind: "ready" }
  | { kind: "loopback-only" }
  | { kind: "no-network" }
  | { kind: "not-advertising" };

/** What, if anything, stops other devices from reaching this server. */
export function hostReachability(info: HostInfo): HostReachability {
  if (!info.networkAccess) return { kind: "loopback-only" };
  if (!info.addresses.length && !info.publicUrl) return { kind: "no-network" };
  if (!info.advertising) return { kind: "not-advertising" };
  return { kind: "ready" };
}

/**
 * Draws a QR matrix (row-major, 1 = dark) as one SVG path of unit squares,
 * merging horizontal runs so the path stays small.
 */
export function qrPath(matrix: ArrayLike<number>): {
  size: number;
  path: string;
} {
  const size = Math.round(Math.sqrt(matrix.length));
  if (!size || size * size !== matrix.length) return { size: 0, path: "" };
  const parts: string[] = [];
  for (let y = 0; y < size; y += 1) {
    let x = 0;
    while (x < size) {
      if (!matrix[y * size + x]) {
        x += 1;
        continue;
      }
      const start = x;
      while (x < size && matrix[y * size + x]) x += 1;
      parts.push(`M${start} ${y}h${x - start}v1h-${x - start}z`);
    }
  }
  return { size, path: parts.join("") };
}
