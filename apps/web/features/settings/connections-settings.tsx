"use client";

import DesktopServerCard from "@/features/server/desktop-server-card";
import HostPanel from "@/features/server/host-panel";
import RemoteAccessPanel from "@/features/server/remote-access-panel";
import ServerConnectionPanel from "@/features/server/server-connection-panel";

export default function ConnectionsSettings() {
  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-base font-semibold text-white">Connections</h2>
        <p className="mt-1 max-w-xl text-sm text-zinc-500">
          One Mujin Music server holds your library; every app (desktop, phone,
          browser) is a remote for it. Connect more devices here.
        </p>
      </div>
      <DesktopServerCard />
      <HostPanel />
      <RemoteAccessPanel />
      <ServerConnectionPanel />
    </div>
  );
}
