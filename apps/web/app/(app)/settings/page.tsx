"use client";

import BitrateForm from "@/features/settings/bitrate-form";
import AccountSettings from "@/features/settings/account-settings";
import AdvancedSettings from "@/features/settings/advanced-settings";
import LibrarySettings from "@/features/settings/library-settings";
import UserForm from "@/features/settings/user-form";
import { useSession } from "@/features/account/session-provider";
import { useEffect, useState } from "react";
import AudioOutputSettings from "@/features/settings/audio-output-settings";
import DataSettings from "@/features/settings/data-settings";
import UpdateSettings from "@/features/settings/update-settings";
import EnrichmentSettings from "@/features/settings/enrichment-settings";
import MetadataSourcesSettings from "@/features/settings/metadata-sources-settings";
import RadioSettings from "@/features/settings/radio-settings";

const settingsTabs = [
  "account",
  "playback",
  "outputs",
  "library",
  "data",
  "users",
  "advanced",
] as const;
type SettingsTab = (typeof settingsTabs)[number];

const labels: Record<SettingsTab, string> = {
  account: "Account",
  playback: "Playback",
  outputs: "Audio outputs",
  library: "Library",
  data: "Data",
  users: "Users",
  advanced: "Advanced",
};

export default function SettingsPage() {
  const { session } = useSession();
  const isAdmin = session?.role === "admin";
  const [activeTab, setActiveTab] = useState<SettingsTab>("account");
  const availableTabs: readonly SettingsTab[] = isAdmin
    ? settingsTabs
    : (["account", "playback", "outputs", "data"] as const);
  useEffect(() => {
    const requested = new URLSearchParams(window.location.search).get("tab");
    const allowed: readonly string[] = isAdmin
      ? settingsTabs
      : ["account", "playback", "outputs", "data"];
    if (requested && allowed.includes(requested)) {
      setActiveTab(requested as SettingsTab);
    }
  }, [isAdmin]);

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
          {activeTab === "data" && <DataSettings isAdmin={isAdmin} />}
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
