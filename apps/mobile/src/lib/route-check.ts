/**
 * Checks, from the phone, whether an address reaches a given Parson library,
 * the same check the server runs in Settings → Connections.
 */
export type RouteCheck =
  | { ok: true; latencyMs: number }
  | { ok: false; reason: "unreachable" | "other-library" | "not-parson" };

export async function checkRoute(
  origin: string,
  instanceId: string,
  {
    fetcher = fetch,
    timeoutMs = 5_000,
    now = () => Date.now(),
  }: { fetcher?: typeof fetch; timeoutMs?: number; now?: () => number } = {},
): Promise<RouteCheck> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  const started = now();
  try {
    const response = await fetcher(`${origin}/.well-known/parson`, {
      headers: { Accept: "application/json" },
      signal: controller.signal,
    });
    if (!response.ok) return { ok: false, reason: "not-parson" };
    const manifest = (await response.json()) as {
      protocol?: unknown;
      instanceId?: unknown;
    };
    if (manifest.protocol !== "parson")
      return { ok: false, reason: "not-parson" };
    if (manifest.instanceId !== instanceId)
      return { ok: false, reason: "other-library" };
    return { ok: true, latencyMs: Math.max(0, now() - started) };
  } catch {
    return { ok: false, reason: "unreachable" };
  } finally {
    clearTimeout(timer);
  }
}

/** "music-nas.tail1234.ts.net" → "Tailscale"; otherwise the host. */
export function routeLabel(origin: string): string {
  try {
    const host = new URL(origin).hostname;
    if (
      host.endsWith(".ts.net") ||
      /^100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\./.test(host)
    )
      return "Tailscale";
    return host;
  } catch {
    return origin;
  }
}

export function routeCheckText(check: RouteCheck): string {
  if (check.ok) return `Works · ${check.latencyMs} ms`;
  switch (check.reason) {
    case "other-library":
      return "Reaches a different library";
    case "not-parson":
      return "Something else answered";
    default:
      return "No answer from here";
  }
}
