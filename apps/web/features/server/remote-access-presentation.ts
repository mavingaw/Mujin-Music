import type { PathCheck, RouterState } from "@parson/music-sdk";

export type RouterSummary = {
  tone: "good" | "warn" | "off" | "bad";
  title: string;
  detail: string;
};

/** What the router port forward is doing, in plain words. */
export function describeRouter(router: RouterState): RouterSummary {
  const where =
    router.external_ip && router.external_port
      ? `${router.external_ip}:${router.external_port}`
      : "";
  switch (router.status) {
    case "forwarded":
      return {
        tone: "good",
        title: `Port ${router.external_port} forwarded by ${router.router_name ?? "your router"}`,
        detail: `Reachable at ${where}${router.internal_address ? `, forwarded to ${router.internal_address}` : ""}. ${
          router.lease_seconds === 0
            ? "Your router only allows permanent forwards; Parson removes it when you switch this off."
            : "Parson renews it while the server runs; it lapses on its own if the server stops."
        }`,
      };
    case "unreachable":
      return {
        tone: "warn",
        title: "Forwarded, but not reachable from the internet",
        detail:
          router.message ??
          "Your router’s own address is shared or private, so the forward can’t be reached from outside.",
      };
    case "failed":
      return {
        tone: "bad",
        title: "The router did not forward the port",
        detail:
          router.message ??
          "Turn on UPnP on your router, or forward the port by hand.",
      };
    case "working":
      return {
        tone: "off",
        title: "Asking your router…",
        detail: "Looking for a router with UPnP on this network.",
      };
    default:
      return {
        tone: "off",
        title: "Router port forwarding is off",
        detail: "Parson can ask your router (UPnP) to forward a port to it.",
      };
  }
}

/** Short badge text for one check result. */
export function outcomeLabel(result: PathCheck): string {
  switch (result.outcome) {
    case "reachable":
      return result.latency_ms != null ? `${result.latency_ms} ms` : "Works";
    case "other_server":
      return "Other server";
    case "not_parson":
      return "Not Parson";
    default:
      return "No answer";
  }
}
