import api from "../core/http";

/**
 * Reaching the server from outside the home: a VPN (Tailscale/WireGuard),
 * an address the owner set up (HTTPS reverse proxy, dynamic DNS) or a router
 * port forward that the server sets up with UPnP. Administrators only.
 */
export type RemoteAccessMode = "off" | "router" | "manual";

export type RemoteAccessConfig = {
  mode: RemoteAccessMode;
  manual_url: string | null;
  external_port: number | null;
};

export type RouterStatus =
  "off" | "working" | "forwarded" | "unreachable" | "failed";

export type RouterState = {
  status: RouterStatus;
  router_name: string | null;
  external_ip: string | null;
  external_port: number | null;
  internal_address: string | null;
  /** 0 when the router only allows permanent forwards. */
  lease_seconds: number | null;
  updated_at: string | null;
  message: string | null;
};

export type RemoteAddressKind = "vpn" | "manual" | "router";

export type RemoteAddress = {
  origin: string;
  kind: RemoteAddressKind;
  label: string;
  /** Outcome of the last check, or null when not checked yet. */
  verified: boolean | null;
};

export type CheckOutcome =
  "reachable" | "unreachable" | "other_server" | "not_parson";

export type PathCheck = {
  origin: string;
  kind: RemoteAddressKind;
  label: string;
  outcome: CheckOutcome;
  latency_ms: number | null;
  detail: string;
};

export type CheckReport = {
  checked_at: string;
  results: PathCheck[];
};

export type RemoteAccessStatus = {
  config: RemoteAccessConfig;
  router: RouterState;
  addresses: RemoteAddress[];
  tailscale_name: string | null;
  last_check: CheckReport | null;
  network_access: boolean;
};

export async function getRemoteAccess(): Promise<RemoteAccessStatus> {
  return (await api.get<RemoteAccessStatus>("/remote-access")).data;
}

export async function updateRemoteAccess(
  config: Partial<RemoteAccessConfig> & Pick<RemoteAccessConfig, "mode">,
): Promise<RemoteAccessStatus> {
  return (
    await api.put<RemoteAccessStatus>("/remote-access", config, {
      timeout: 30_000,
    })
  ).data;
}

export async function checkRemoteAccess(): Promise<CheckReport> {
  return (
    await api.post<CheckReport>("/remote-access/check", undefined, {
      timeout: 30_000,
    })
  ).data;
}

export type RemoteAccessSummary = {
  tone: "good" | "warn" | "off";
  title: string;
  detail: string;
};

/** One line for the Connections page: is the library reachable away from home? */
export function summarizeRemoteAccess(
  status: RemoteAccessStatus,
): RemoteAccessSummary {
  const check = status.last_check;
  const best = check?.results.find((result) => result.outcome === "reachable");
  if (!status.addresses.length) {
    if (status.config.mode === "router") {
      if (status.router.status === "working") {
        return {
          tone: "off",
          title: "Setting up…",
          detail: "Asking your router to forward a port.",
        };
      }
      return {
        tone: "warn",
        title: "Not reachable away from home",
        detail:
          status.router.message ??
          "The router did not set up the port forward.",
      };
    }
    return {
      tone: "off",
      title: "Home network only",
      detail:
        "Parson apps reach this library on your home network. Choose a way in below to listen away from home.",
    };
  }
  if (!check) {
    return {
      tone: "off",
      title: "Not checked yet",
      detail: "Run a check to confirm each address reaches this server.",
    };
  }
  if (best) {
    return {
      tone: "good",
      title: "Reachable away from home",
      detail: `${best.label} works${best.latency_ms != null ? ` (${best.latency_ms} ms)` : ""}. The Parson app switches to it when you leave home.`,
    };
  }
  return {
    tone: "warn",
    title: "No address passed the check",
    detail: check.results[0]?.detail ?? "None of the addresses answered.",
  };
}
