"use client";

import BitrateForm from "@/features/settings/bitrate-form";
import AccountSettings from "@/features/settings/account-settings";
import AboutSettings from "@/features/settings/about-settings";
import AdvancedSettings from "@/features/settings/advanced-settings";
import LibrarySettings from "@/features/settings/library-settings";
import UserForm from "@/features/settings/user-form";
import { useSession } from "@/features/account/session-provider";
import { Suspense, useState } from "react";
import AudioOutputSettings from "@/features/settings/audio-output-settings";
import DisplaysSettings from "@/features/settings/displays-settings";
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
import ProfilesSettings from "@/features/profiles/profiles-settings";
import ScrobblingSettings from "@/features/scrobbling/scrobbling-settings";
import BetterVersionsSettings from "@/features/settings/better-versions-settings";
import AppearanceSettings from "@/features/appearance/appearance-settings";

const settingsTabs = [
  "account",
  "appearance",
  "profiles",
  "scrobbling",
  "playback",
  "outputs",
  "displays",
  "connections",
  "services",
  "library",
  "better-versions",
  "data",
  "home-assistant",
  "users",
  "advanced",
  "about",
] as const;
type SettingsTab = (typeof settingsTabs)[number];

const labels: Record<SettingsTab, string> = {
  account: "Account",
  appearance: "Appearance",
  profiles: "Profiles",
  scrobbling: "Scrobbling",
  playback: "Playback",
  outputs: "Audio outputs",
  displays: "Displays",
  connections: "Connections",
  services: "Services",
  library: "Library",
  "better-versions": "Better versions",
  data: "Data",
  "home-assistant": "Home Assistant",
  users: "Users",
  advanced: "Advanced",
  about: "About",
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
        "appearance",
        "profiles",
        "scrobbling",
        "playback",
        "outputs",
        "displays",
        "connections",
        "services",
        "data",
        "home-assistant",
        "about",
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
    <div className="mx-auto w-full max-w-[1180px] px-5 pb-36 pt-8 sm:px-8 md:pt-12 lg:px-12">
      <p className="mj-eyebrow mb-2">Mujin Music</p>
      <h1 className="mj-display mb-10 text-[2.5rem] text-mujin-text md:text-[3.25rem]">
        Settings
      </h1>
      <section className="grid items-start gap-8 md:grid-cols-[200px_minmax(0,1fr)] lg:gap-12">
        <nav
          aria-label="Settings sections"
          className="flex gap-1 overflow-x-auto border-b border-white/[0.08] pb-2 md:sticky md:top-6 md:grid md:border-0 md:border-r md:pb-0 md:pr-4"
        >
          {availableTabs.map((tab) => (
            <button
              aria-current={activeTab === tab ? "page" : undefined}
              className={`relative shrink-0 rounded-lg px-3 py-2 text-left text-[0.8438rem] font-medium transition-colors ${
                activeTab === tab
                  ? "bg-mujin-copper/[0.12] text-mujin-text"
                  : "text-mujin-text-tertiary hover:bg-white/[0.04] hover:text-mujin-text"
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
          {activeTab === "appearance" && <AppearanceSettings />}
          {activeTab === "profiles" && <ProfilesSettings />}
          {activeTab === "scrobbling" && <ScrobblingSettings />}
          {activeTab === "outputs" && <AudioOutputSettings isAdmin={isAdmin} />}
          {activeTab === "displays" && <DisplaysSettings />}
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
          {isAdmin && activeTab === "better-versions" && (
            <BetterVersionsSettings />
          )}
          {isAdmin && activeTab === "users" && <UserForm />}
          {isAdmin && activeTab === "advanced" && <AdvancedSettings />}
          {activeTab === "about" && <AboutSettings />}
        </div>
      </section>
    </div>
  );
}
