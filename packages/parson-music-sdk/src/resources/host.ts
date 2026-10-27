import api from "../core/http";

/**
 * The server ("core") side of Parson's server/remote split. Any Parson app
 * can ask the server it is connected to how other devices reach it, then
 * show those addresses and a QR code so a phone can join in one step.
 */
export type HostAddressKind = "lan" | "vpn" | "link-local" | "public";

export type HostAddress = {
  address: string;
  origin: string;
  interface: string;
  kind: HostAddressKind;
};

/** An address for away from home (VPN, the owner's own URL, a port forward). */
export type RemoteHostAddress = {
  origin: string;
  kind: "vpn" | "manual" | "router";
  label: string;
  /** Outcome of the server's last check of this address; null if unchecked. */
  verified: boolean | null;
};

export type HostInfo = {
  instanceId: string;
  name: string;
  serverVersion: string;
  port: number;
  bootId: string;
  startedAt: number;
  /** False when the server only listens on this computer (loopback). */
  networkAccess: boolean;
  /** True while the server announces itself with `_parson._tcp` mDNS. */
  advertising: boolean;
  publicUrl: string | null;
  addresses: HostAddress[];
  /** Addresses the phone app saves for when it is away from home. */
  remote: RemoteHostAddress[];
};

export type ConnectLink = {
  origin: string;
  name: string | null;
  instanceId: string | null;
  /** Addresses for away from home carried by the link. */
  remote: string[];
};

const REMOTE_KINDS = new Set(["vpn", "manual", "router"]);
const MAX_REMOTE_ADDRESSES = 6;

const ADDRESS_KINDS = new Set<HostAddressKind>([
  "lan",
  "vpn",
  "link-local",
  "public",
]);

function httpOrigin(value: unknown): string | null {
  if (typeof value !== "string" || value.length > 512) return null;
  try {
    const url = new URL(value);
    if (url.protocol !== "http:" && url.protocol !== "https:") return null;
    if (url.username || url.password || !url.hostname) return null;
    return url.origin;
  } catch {
    return null;
  }
}

function boundedText(value: unknown, max = 256): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed && trimmed.length <= max ? trimmed : null;
}

export function parseHostInfo(value: unknown): HostInfo | null {
  if (!value || typeof value !== "object") return null;
  const raw = value as Record<string, unknown>;
  const instanceId = boundedText(raw.instanceId);
  const name = boundedText(raw.name) ?? "Parson";
  const port = Number(raw.port);
  if (!instanceId || !Number.isInteger(port) || port < 1 || port > 65_535)
    return null;
  const addresses = Array.isArray(raw.addresses)
    ? raw.addresses.flatMap((entry): HostAddress[] => {
        if (!entry || typeof entry !== "object") return [];
        const address = entry as Record<string, unknown>;
        const origin = httpOrigin(address.origin);
        const kind = address.kind as HostAddressKind;
        if (!origin || !ADDRESS_KINDS.has(kind)) return [];
        return [
          {
            address: boundedText(address.address) ?? new URL(origin).hostname,
            origin,
            interface: boundedText(address.interface) ?? "",
            kind,
          },
        ];
      })
    : [];
  return {
    instanceId,
    name,
    serverVersion: boundedText(raw.serverVersion, 64) ?? "",
    port,
    bootId: boundedText(raw.bootId, 128) ?? "",
    startedAt: Number.isFinite(Number(raw.startedAt))
      ? Number(raw.startedAt)
      : 0,
    networkAccess: raw.networkAccess === true,
    advertising: raw.advertising === true,
    publicUrl: httpOrigin(raw.publicUrl),
    addresses,
    remote: Array.isArray(raw.remote)
      ? raw.remote
          .flatMap((entry): RemoteHostAddress[] => {
            if (!entry || typeof entry !== "object") return [];
            const remote = entry as Record<string, unknown>;
            const origin = httpOrigin(remote.origin);
            if (!origin || !REMOTE_KINDS.has(String(remote.kind))) return [];
            return [
              {
                origin,
                kind: remote.kind as RemoteHostAddress["kind"],
                label: boundedText(remote.label, 64) ?? "Away from home",
                verified:
                  typeof remote.verified === "boolean" ? remote.verified : null,
              },
            ];
          })
          .slice(0, MAX_REMOTE_ADDRESSES)
      : [],
  };
}

/**
 * True for a host that can only be reached at home: a private, loopback,
 * link-local, carrier-grade NAT or documentation IP address, or a `.local`
 * style name. VPN addresses are private too, but are meant for devices on
 * the VPN, so callers decide about those.
 */
export function isHomeOnlyHost(host: string): boolean {
  const bare = host.replace(/^\[|\]$/g, "").toLowerCase();
  if (
    bare === "localhost" ||
    /\.(local|lan|home|home\.arpa|internal|localdomain|localhost)\.?$/.test(
      bare,
    )
  )
    return true;
  const v4 = bare.replace(/^::ffff:/, "").match(/^(\d+)\.(\d+)\.(\d+)\.(\d+)$/);
  if (v4) {
    const [a = 0, b = 0, c = 0] = v4.slice(1, 4).map(Number);
    return (
      a === 0 ||
      a === 10 ||
      a === 127 ||
      a >= 224 ||
      (a === 100 && b >= 64 && b < 128) ||
      (a === 169 && b === 254) ||
      (a === 172 && b >= 16 && b < 32) ||
      (a === 192 && b === 168) ||
      (a === 192 && b === 0 && (c === 0 || c === 2)) ||
      (a === 198 && (b === 18 || b === 19)) ||
      (a === 198 && b === 51 && c === 100) ||
      (a === 203 && b === 0 && c === 113)
    );
  }
  if (bare.includes(":")) {
    return (
      bare === "::1" ||
      bare === "::" ||
      /^f[cd]/.test(bare) ||
      /^fe[89ab]/.test(bare) ||
      bare.startsWith("2001:db8:")
    );
  }
  return !bare.includes(".");
}

/**
 * Addresses worth saving for away from home: everything except ones the
 * server checked and found unreachable, and never an address that only works
 * at home (a LAN address is not a way in from outside).
 */
export function usableRemoteOrigins(info: Pick<HostInfo, "remote">): string[] {
  return [
    ...new Set(
      info.remote
        .filter((address) => address.verified !== false)
        .filter(
          (address) =>
            address.kind === "vpn" ||
            !isHomeOnlyHost(new URL(address.origin).hostname),
        )
        .map((address) => address.origin),
    ),
  ];
}

export async function getHostInfo(): Promise<HostInfo | null> {
  const response = await api.get<unknown>("/host", { timeout: 5_000 });
  return parseHostInfo(response.data);
}

/**
 * Every origin another device could use, best first: an explicitly
 * configured public URL, then the server's ranked network addresses.
 */
export function hostOrigins(info: HostInfo): string[] {
  const origins = [
    ...(info.publicUrl ? [info.publicUrl] : []),
    ...info.addresses.map((address) => address.origin),
  ];
  return [...new Set(origins)];
}

export function hostAddressLabel(address: HostAddress): string {
  switch (address.kind) {
    case "lan":
      return "Home network";
    case "vpn":
      return "VPN";
    case "link-local":
      return "Direct cable";
    default:
      return "Public address";
  }
}

/**
 * The text a QR code carries. `app` opens the Parson phone app straight on
 * this server (`parson://connect`); `browser` opens the server's web app,
 * which works on any phone without installing anything.
 */
export function connectLink(
  info: Pick<HostInfo, "instanceId" | "name"> &
    Partial<Pick<HostInfo, "remote">>,
  origin: string,
  target: "app" | "browser",
): string {
  const normalized = httpOrigin(origin);
  if (!normalized) return "";
  if (target === "browser") return `${normalized}/`;
  const params = new URLSearchParams({
    server: normalized,
    name: info.name,
    id: info.instanceId,
  });
  // The phone also learns how to reach the library away from home.
  for (const remote of info.remote
    ? usableRemoteOrigins({ remote: info.remote })
    : []) {
    if (remote !== normalized) params.append("remote", remote);
  }
  return `parson://connect?${params.toString()}`;
}

/** Reads a `parson://connect?...` link (from a QR code or a shared link). */
export function parseConnectLink(value: string): ConnectLink | null {
  let url: URL;
  try {
    url = new URL(value.trim());
  } catch {
    return null;
  }
  if (url.protocol !== "parson:") return null;
  // `parson://connect?..` parses with host "connect"; `parson:///connect` and
  // `parson:connect` land in the pathname instead.
  const route = `${url.host}${url.pathname}`.replace(/^\/+|\/+$/g, "");
  if (route !== "connect") return null;
  const origin = httpOrigin(url.searchParams.get("server"));
  if (!origin) return null;
  return {
    origin,
    name: boundedText(url.searchParams.get("name")),
    instanceId: boundedText(url.searchParams.get("id")),
    remote: [
      ...new Set(
        url.searchParams
          .getAll("remote")
          .map(httpOrigin)
          .filter(
            (value): value is string => Boolean(value) && value !== origin,
          ),
      ),
    ].slice(0, MAX_REMOTE_ADDRESSES),
  };
}

/** "3 hours", "2 days": how long the server has been running. */
export function hostUptime(startedAt: number, now = Date.now()): string {
  if (!startedAt) return "";
  const seconds = Math.max(0, Math.floor(now / 1000 - startedAt));
  const units: [number, string][] = [
    [86_400, "day"],
    [3_600, "hour"],
    [60, "minute"],
  ];
  for (const [size, unit] of units) {
    if (seconds >= size) {
      const count = Math.floor(seconds / size);
      return `${count} ${unit}${count === 1 ? "" : "s"}`;
    }
  }
  return "less than a minute";
}
