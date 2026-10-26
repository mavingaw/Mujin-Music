"use client";

import BitrateForm from "@/features/settings/bitrate-form";
import AccountSettings from "@/features/settings/account-settings";
import AdvancedSettings from "@/features/settings/advanced-settings";
import LibrarySettings from "@/features/settings/library-settings";
import UserForm from "@/features/settings/user-form";
import { useSession } from "@/features/account/session-provider";
import { Suspense, useState } from "react";
import AudioOutputSettings from "@/features/settings/audio-output-settings";
import DataSettings from "@/features/settings/data-settings";
import UpdateSettings from "@/features/settings/update-settings";
import EnrichmentSettings from "@/features/settings/enrichment-settings";
import MetadataSourcesSettings from "@/features/settings/metadata-sources-settings";
import RadioSettings from "@/features/settings/radio-settings";
import MusicServiceConnections from "@/features/imports/connections-settings";
import ConnectionsSettings from "@/features/settings/connections-settings";
import ServicesSettings from "@/features/streaming/services-settings";
import { useSearchParams } from "next/navigation";
import HomeAssistantSettings from "@/features/settings/home-assistant-settings";

const settingsTabs = [
  "account",
  "playback",
  "outputs",
  "connections",
  "services",
  "library",
  "data",
  "home-assistant",
  "users",
  "advanced",
] as const;
type SettingsTab = (typeof settingsTabs)[number];

const labels: Record<SettingsTab, string> = {
  account: "Account",
  playback: "Playback",
  outputs: "Audio outputs",
  connections: "Connections",
  services: "Services",
  library: "Library",
  data: "Data",
  "home-assistant": "Home Assistant",
  users: "Users",
  advanced: "Advanced",
};

export default function SettingsPage() {
  return (
    <Suspense fallback={null}>
      <SettingsContent />
    </Suspense>
  );
}

function SettingsContent() {
  const { session } = useSession();
  const isAdmin = session?.role === "admin";
  const requestedTab = useSearchParams().get("tab");
  const allowedTabs: readonly SettingsTab[] = isAdmin
    ? settingsTabs
    : ([
        "account",
        "playback",
        "outputs",
        "connections",
        "services",
        "data",
        "home-assistant",
      ] as const);
  const [activeTab, setActiveTab] = useState<SettingsTab>(() =>
    allowedTabs.includes(requestedTab as SettingsTab)
      ? (requestedTab as SettingsTab)
      : "account",
  );
  // Deep links such as /settings?tab=outputs from the signal path; an
  // admin tab applies once the session says the user is an admin.
  const requestKey = `${requestedTab ?? ""}|${isAdmin}`;
  const [appliedKey, setAppliedKey] = useState(requestKey);
  if (requestKey !== appliedKey) {
    setAppliedKey(requestKey);
    if (allowedTabs.includes(requestedTab as SettingsTab))
      setActiveTab(requestedTab as SettingsTab);
  }
  const availableTabs = allowedTabs;

  return (
    <div className="mx-auto w-full max-w-250 px-5 py-9 pb-36 sm:px-7">
      <h1 className="mb-9 text-3xl font-semibold text-white">Settings</h1>
      <section className="grid items-start gap-8 md:grid-cols-[170px_minmax(0,1fr)]">
        <nav
          aria-label="Settings sections"
          className="flex gap-1 overflow-x-auto border-b border-white/8 pb-2 md:grid md:border-0 md:pb-0"
        >
          {availableTabs.map((tab) => (
            <button
              aria-current={activeTab === tab ? "page" : undefined}
              className={`shrink-0 rounded-md px-3 py-2 text-left text-sm font-medium transition-colors ${
                activeTab === tab
                  ? "bg-white/8 text-white"
                  : "text-zinc-500 hover:bg-white/4 hover:text-zinc-200"
              }`}
              key={tab}
              onClick={() => setActiveTab(tab)}
              type="button"
            >
              {labels[tab]}
            </button>
          ))}
        </nav>

        <div className="min-w-0" id={`settings-panel-${activeTab}`}>
          {activeTab === "account" && (
            <>
              <AccountSettings />
              <UpdateSettings />
            </>
          )}
          {activeTab === "playback" && (
            <>
              <BitrateForm initialBitrate={session?.bitrate ?? 0} />
              <RadioSettings />
            </>
          )}
          {activeTab === "outputs" && <AudioOutputSettings isAdmin={isAdmin} />}
          {activeTab === "connections" && (
            <>
              <ConnectionsSettings />
              <div className="mt-10">
                <MusicServiceConnections />
              </div>
            </>
          )}
          {activeTab === "services" && <ServicesSettings />}
          {activeTab === "data" && <DataSettings isAdmin={isAdmin} />}
          {activeTab === "home-assistant" && <HomeAssistantSettings />}
          {isAdmin && activeTab === "library" && (
            <>
              <LibrarySettings />
              <div className="mt-8">
                <EnrichmentSettings />
              </div>
              <div className="mt-8">
                <MetadataSourcesSettings />
              </div>
            </>
          )}
          {isAdmin && activeTab === "users" && <UserForm />}
          {isAdmin && activeTab === "advanced" && <AdvancedSettings />}
        </div>
      </section>
    </div>
  );
}
