/**
 * Which server a sign-in page belongs to. The web app is served by the
 * server it signs in to, so its public manifest names it; the address is
 * where the browser reached it.
 */
export type ServerIdentity = { name: string | null; instanceId: string | null };

export async function readServerIdentity(
  fetcher: typeof fetch = fetch,
): Promise<ServerIdentity> {
  const response = await fetcher("/.well-known/parson", {
    cache: "no-store",
    headers: { Accept: "application/json" },
  });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  const manifest = (await response.json()) as {
    name?: unknown;
    instanceId?: unknown;
  };
  const text = (value: unknown) =>
    typeof value === "string" && value.trim() && value.length <= 256
      ? value.trim()
      : null;
  return { name: text(manifest.name), instanceId: text(manifest.instanceId) };
}

/**
 * "Sign in to Basement NAS" over "192.168.1.20:1993". The server's own name
 * wins over a name carried in the link (`?library=`), which only stands in
 * while the manifest loads or when it can't be read.
 */
export function signInHeading({
  serverName,
  requestedName,
  host,
}: {
  serverName: string | null;
  requestedName: string | null;
  host: string;
}): { title: string; address: string } {
  const name = serverName?.trim() || requestedName?.trim() || "";
  return {
    title: name ? `Sign in to ${name}` : "Sign in",
    address: host,
  };
}
