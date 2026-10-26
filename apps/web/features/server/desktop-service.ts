import type { DesktopServiceState } from "@/lib/desktop/bridge";

export type ServiceSummary = {
  running: boolean;
  status: string;
  detail: string;
  autostartLabel: string;
};

const MECHANISM: Record<string, { where: string; autostart: string }> = {
  "windows-tray": {
    where: "Parson Server in the notification area",
    autostart: "Start with Windows",
  },
  "launch-agent": {
    where: "Parson Server in the menu bar",
    autostart: "Launch at startup",
  },
  "systemd-user": {
    where: "systemd user service parson-server",
    autostart: "Start when you sign in",
  },
  "xdg-autostart": {
    where: "Parson Server background app (XDG autostart)",
    autostart: "Start when you sign in",
  },
};

/** What the desktop's server card says about the background service. */
export function serviceSummary(
  service: DesktopServiceState | null,
): ServiceSummary | null {
  if (!service?.available || !service.kind) return null;
  const mechanism = MECHANISM[service.kind];
  if (!mechanism) return null;
  const running = Boolean(service.running);
  return {
    running,
    status: running ? "Running in the background" : "Stopped",
    detail: running
      ? mechanism.where
      : "Phones and other computers can’t reach your library",
    autostartLabel: mechanism.autostart,
  };
}
