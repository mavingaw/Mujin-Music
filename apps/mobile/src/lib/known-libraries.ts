/**
 * Parson servers this phone has connected to, so switching between them
 * (home NAS, the desktop in the studio) is one tap and each keeps its own
 * signed-in accounts.
 */
export type KnownLibrary = {
  instanceId: string;
  origin: string;
  name: string;
  lastConnectedAt: number;
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
    Number.isFinite(library.lastConnectedAt)
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
  const next = { ...library, lastConnectedAt: now };
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
