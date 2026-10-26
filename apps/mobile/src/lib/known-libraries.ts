/**
 * Parson servers this phone has connected to, so switching between them
 * (home NAS, the desktop in the studio) is one tap and each keeps its own
 * signed-in accounts.
 */
export type KnownLibrary = {
  instanceId: string;
  /** The home-network address. */
  origin: string;
  name: string;
  lastConnectedAt: number;
  /**
   * Addresses for away from home (Tailscale, the owner's own HTTPS address,
   * a router port forward). Only ever learned from the signed-in server
   * itself, so this phone's sign-in is only sent where that server said.
   */
  remoteOrigins?: string[];
};

export type LibraryRow = KnownLibrary & {
  /** Saved on this phone. */
  known: boolean;
  /** Answering on the network right now. */
  nearby: boolean;
  /** The library the app is using. */
  current: boolean;
  /** Saved accounts for this library on this phone. */
  accounts: number;
};

const MAX_LIBRARIES = 12;
const MAX_REMOTE_ORIGINS = 6;

function validOrigin(value: unknown): value is string {
  if (typeof value !== "string" || value.length > 512) return false;
  try {
    const url = new URL(value);
    return (
      (url.protocol === "http:" || url.protocol === "https:") &&
      !url.username &&
      !url.password &&
      url.origin === value
    );
  } catch {
    return false;
  }
}

function validLibrary(value: unknown): value is KnownLibrary {
  if (!value || typeof value !== "object") return false;
  const library = value as Partial<KnownLibrary>;
  return (
    typeof library.instanceId === "string" &&
    library.instanceId.length > 0 &&
    library.instanceId.length <= 256 &&
    validOrigin(library.origin) &&
    typeof library.name === "string" &&
    library.name.trim().length > 0 &&
    library.name.length <= 256 &&
    typeof library.lastConnectedAt === "number" &&
    Number.isFinite(library.lastConnectedAt) &&
    (library.remoteOrigins === undefined ||
      (Array.isArray(library.remoteOrigins) &&
        library.remoteOrigins.length <= MAX_REMOTE_ORIGINS &&
        library.remoteOrigins.every(validOrigin)))
  );
}

export function parseKnownLibraries(serialized: string | null) {
  if (!serialized) return [];
  try {
    const parsed: unknown = JSON.parse(serialized);
    if (!Array.isArray(parsed)) return [];
    const seen = new Set<string>();
    return parsed
      .filter(validLibrary)
      .filter((library) => {
        if (seen.has(library.instanceId)) return false;
        seen.add(library.instanceId);
        return true;
      })
      .slice(0, MAX_LIBRARIES);
  } catch {
    return [];
  }
}

/** Saves a library after connecting, following it to a new address. */
export function rememberLibrary(
  libraries: KnownLibrary[],
  library: Omit<KnownLibrary, "lastConnectedAt">,
  now = Date.now(),
): KnownLibrary[] {
  const existing = libraries.find(
    (item) => item.instanceId === library.instanceId,
  );
  const remoteOrigins = (
    library.remoteOrigins ??
    existing?.remoteOrigins ??
    []
  ).filter((origin) => origin !== library.origin);
  const next: KnownLibrary = {
    ...library,
    lastConnectedAt: now,
    ...(remoteOrigins.length ? { remoteOrigins } : {}),
  };
  if (!remoteOrigins.length) delete next.remoteOrigins;
  if (!validLibrary(next)) return libraries;
  return [
    next,
    ...libraries.filter(
      (item) =>
        item.instanceId !== library.instanceId &&
        item.origin !== library.origin,
    ),
  ].slice(0, MAX_LIBRARIES);
}

export function forgetLibrary(libraries: KnownLibrary[], instanceId: string) {
  return libraries.filter((library) => library.instanceId !== instanceId);
}

/**
 * Saved libraries merged with what discovery sees right now. A saved library
 * found at a new address is shown (and connected) at that address.
 */
export function libraryRows({
  known,
  nearby,
  currentInstanceId,
  accountInstanceIds = [],
}: {
  known: KnownLibrary[];
  nearby: { instanceId: string; origin: string; name: string }[];
  currentInstanceId: string | null;
  accountInstanceIds?: string[];
}): LibraryRow[] {
  const accounts = (instanceId: string) =>
    accountInstanceIds.filter((id) => id === instanceId).length;
  const rows = new Map<string, LibraryRow>();
  for (const library of known) {
    rows.set(library.instanceId, {
      ...library,
      known: true,
      nearby: false,
      current: library.instanceId === currentInstanceId,
      accounts: accounts(library.instanceId),
    });
  }
  for (const found of nearby) {
    const existing = rows.get(found.instanceId);
    rows.set(found.instanceId, {
      instanceId: found.instanceId,
      origin: found.origin,
      name: found.name || existing?.name || found.origin,
      lastConnectedAt: existing?.lastConnectedAt ?? 0,
      known: Boolean(existing),
      nearby: true,
      current: found.instanceId === currentInstanceId,
      accounts: accounts(found.instanceId),
    });
  }
  return [...rows.values()].sort(
    (left, right) =>
      Number(right.current) - Number(left.current) ||
      Number(right.known) - Number(left.known) ||
      Number(right.nearby) - Number(left.nearby) ||
      right.lastConnectedAt - left.lastConnectedAt ||
      left.name.localeCompare(right.name),
  );
}

export function libraryRowDetail(row: LibraryRow, now = Date.now()) {
  const host = (() => {
    try {
      return new URL(row.origin).host;
    } catch {
      return row.origin;
    }
  })();
  const parts = [host];
  if (row.accounts === 1) parts.push("1 account");
  else if (row.accounts > 1) parts.push(`${row.accounts} accounts`);
  if (row.known && !row.nearby && row.lastConnectedAt) {
    const days = Math.floor((now - row.lastConnectedAt) / 86_400_000);
    parts.push(
      days < 1
        ? "used today"
        : days === 1
          ? "used yesterday"
          : `used ${days} days ago`,
    );
  }
  return parts.join(" · ");
}

/** The home address first, then the away-from-home addresses. */
export function libraryRoutes(library: KnownLibrary): string[] {
  return [...new Set([library.origin, ...(library.remoteOrigins ?? [])])];
}

/** True when `origin` is one of the library's away-from-home addresses. */
export function isRemoteRoute(
  library: KnownLibrary | undefined,
  origin: string,
) {
  return Boolean(
    library &&
    origin !== library.origin &&
    library.remoteOrigins?.includes(origin),
  );
}

/**
 * Whether this phone may send `instanceId`'s sign-in to `origin`: only the
 * home address and the addresses the signed-in server handed out.
 */
export function isTrustedRoute(
  libraries: KnownLibrary[],
  instanceId: string,
  origin: string,
) {
  const library = libraries.find((item) => item.instanceId === instanceId);
  return Boolean(library && libraryRoutes(library).includes(origin));
}

/** Stores the away-from-home addresses the server reported. */
export function setRemoteOrigins(
  libraries: KnownLibrary[],
  instanceId: string,
  origins: string[],
): KnownLibrary[] {
  const index = libraries.findIndex((item) => item.instanceId === instanceId);
  if (index < 0) return libraries;
  const library = libraries[index]!;
  const next = [...new Set(origins)]
    .filter((origin) => validOrigin(origin) && origin !== library.origin)
    .slice(0, MAX_REMOTE_ORIGINS);
  const current = library.remoteOrigins ?? [];
  if (
    next.length === current.length &&
    next.every((origin, position) => origin === current[position])
  )
    return libraries;
  const updated: KnownLibrary = { ...library, remoteOrigins: next };
  if (!next.length) delete updated.remoteOrigins;
  return libraries.map((item, position) =>
    position === index ? updated : item,
  );
}

/** Marks a library as used now without changing its home address. */
export function touchLibrary(
  libraries: KnownLibrary[],
  instanceId: string,
  now = Date.now(),
): KnownLibrary[] {
  return libraries.map((library) =>
    library.instanceId === instanceId
      ? { ...library, lastConnectedAt: now }
      : library,
  );
}

export type RouteProbe<T> = (
  origin: string,
) => Promise<{ instanceId: string; manifest: T }>;

/**
 * Tries every route at once and returns the first one, in route order (home
 * first), that answers as this library. Away from home the home address
 * times out while Tailscale or the owner's address answers.
 */
export async function pickReachableRoute<T>(
  routes: string[],
  instanceId: string,
  probe: RouteProbe<T>,
): Promise<{ origin: string; manifest: T } | null> {
  const results = await Promise.all(
    routes.map(async (origin) => {
      try {
        const answer = await probe(origin);
        return answer.instanceId === instanceId
          ? { origin, manifest: answer.manifest }
          : null;
      } catch {
        return null;
      }
    }),
  );
  return results.find((result) => result !== null) ?? null;
}
