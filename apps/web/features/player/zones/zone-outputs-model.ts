import type { BridgeStatus, ZoneOutputsStatus } from "@parson/music-sdk";

/** One line about a bridge: what it is and how it is doing. */
export function bridgeSummary(bridge: BridgeStatus): {
  title: string;
  detail: string;
  tone: "ok" | "warn" | "off";
} {
  const title = bridge.name ?? bridge.address;
  if (!bridge.connected) {
    return {
      title,
      detail: bridge.last_error
        ? `Not connected: ${bridge.last_error}`
        : "Connecting…",
      tone: "off",
    };
  }
  const parts: string[] = [];
  if (bridge.platform) parts.push(bridge.platform);
  parts.push(bridge.devices === 1 ? "1 output" : `${bridge.devices} outputs`);
  if (bridge.backend) parts.push(bridge.backend);
  const clock = clockLabel(bridge);
  if (clock) parts.push(clock);
  if (bridge.devices === 0 && bridge.note) {
    return { title, detail: bridge.note, tone: "warn" };
  }
  return {
    title,
    detail: parts.join(" · "),
    tone: bridge.last_error ? "warn" : "ok",
  };
}

/** `clock synced ±0.2 ms`: how well the server knows the bridge's clock. */
export function clockLabel(bridge: BridgeStatus): string | null {
  if (bridge.clock_uncertainty_us === null) return null;
  const ms = bridge.clock_uncertainty_us / 1000;
  return `clock synced ±${ms < 0.1 ? "0.1" : ms.toFixed(1)} ms`;
}

/** Counts for the outputs footer. */
export function outputCounts(outputs: ZoneOutputsStatus | null | undefined): {
  bridges: number;
  bridgeOutputs: number;
  airplay: number;
  airplayUnsupported: string[];
  chromecast: number;
} {
  const bridges = outputs?.bridges?.bridges ?? [];
  const receivers = outputs?.airplay?.receivers ?? [];
  return {
    bridges: bridges.filter((bridge) => bridge.connected).length,
    bridgeOutputs: bridges.reduce(
      (total, bridge) => total + (bridge.connected ? bridge.devices : 0),
      0,
    ),
    airplay: receivers.filter((receiver) => receiver.supported).length,
    airplayUnsupported: receivers
      .filter((receiver) => !receiver.supported)
      .map((receiver) => receiver.name),
    chromecast: outputs?.chromecast?.devices.length ?? 0,
  };
}

/** Error text for adding a bridge. */
export function addBridgeError(error: unknown): string {
  const response = (
    error as { response?: { status?: number; data?: { error?: string } } }
  )?.response;
  if (response?.status === 403) return "Only an administrator can add bridges.";
  const code = response?.data?.error ?? "";
  if (code === "invalid_bridge_address")
    return "Enter a host name or address, like pi.local or 192.168.1.20.";
  if (code === "bridges_disabled")
    return "Parson Bridge support is turned off on this server (PARSON_BRIDGES=off).";
  return code || "Could not reach a Parson Bridge at that address.";
}
