"use client";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { getApiErrorMessage } from "@/features/library/tools/tool-errors";
import { RadioSwitch } from "@/features/player/radio/radio-controls";
import {
  checkEnhancerConnection,
  checkFetchConnection,
  clearUpgradeSecret,
  getUpgradeSettings,
  saveUpgradeSecret,
  saveUpgradeSettings,
  type EnhancerCheck,
  type FetchCheck,
  type UpgradeSecret,
  type UpgradeSettings,
  type UpgradeSettingsView,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  AlertTriangle,
  CheckCircle2,
  KeyRound,
  Loader2,
  PlugZap,
  Sparkles,
  XCircle,
} from "lucide-react";
import { useState, type ReactNode } from "react";
import { toast } from "sonner";

const settingsKey = ["upgrades", "settings"] as const;

const selectClass =
  "h-10 w-full rounded-md border border-zinc-800 bg-black px-3 text-sm text-zinc-200 focus-visible:border-zinc-600 focus-visible:outline-none";

function Row({
  label,
  description,
  children,
}: {
  label: string;
  description?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-3 py-4 sm:flex-row sm:items-start sm:justify-between sm:gap-6">
      <div className="min-w-0">
        <p className="text-sm font-medium text-zinc-200">{label}</p>
        {description && (
          <div className="mt-1 max-w-md text-xs leading-5 text-zinc-500">
            {description}
          </div>
        )}
      </div>
      <div className="shrink-0 sm:max-w-xs sm:flex-1 sm:text-right">
        {children}
      </div>
    </div>
  );
}

function SecretField({
  name,
  label,
  saved,
  placeholder,
  onChanged,
}: {
  name: UpgradeSecret;
  label: string;
  saved: boolean;
  placeholder: string;
  onChanged: () => Promise<void>;
}) {
  const [editing, setEditing] = useState(!saved);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const save = async () => {
    setBusy(true);
    try {
      await saveUpgradeSecret(name, value);
      setValue("");
      setEditing(false);
      toast.success(`${label} saved`);
      await onChanged();
    } catch (error) {
      toast(getApiErrorMessage(error, `${label} could not be saved.`));
    } finally {
      setBusy(false);
    }
  };
  if (saved && !editing) {
    return (
      <div className="flex items-center justify-end gap-2">
        <span
          className="inline-flex items-center gap-1.5 text-xs text-quality-high"
          data-secret-saved={name}
        >
          <CheckCircle2 className="h-3.5 w-3.5" />
          Saved
        </span>
        <Button onClick={() => setEditing(true)} size="sm" variant="secondary">
          Replace
        </Button>
        <Button
          onClick={() =>
            void clearUpgradeSecret(name)
              .then(onChanged)
              .catch((error) =>
                toast(getApiErrorMessage(error, "Could not remove it.")),
              )
          }
          size="sm"
          variant="ghost"
        >
          Remove
        </Button>
      </div>
    );
  }
  return (
    <form
      className="flex gap-2"
      onSubmit={(event) => {
        event.preventDefault();
        if (value.trim()) void save();
      }}
    >
      <Input
        aria-label={label}
        autoComplete="off"
        className="font-mono"
        onChange={(event) => setValue(event.target.value)}
        placeholder={placeholder}
        type="password"
        value={value}
      />
      <Button
        disabled={busy || !value.trim()}
        type="submit"
        variant="secondary"
      >
        {busy ? <Loader2 className="animate-spin" /> : <KeyRound />}
        Save
      </Button>
      {saved && (
        <Button onClick={() => setEditing(false)} type="button" variant="ghost">
          Cancel
        </Button>
      )}
    </form>
  );
}

function CheckResult({ check }: { check: FetchCheck }) {
  const time = new Date(check.checked_at * 1000).toLocaleTimeString();
  return (
    <div
      className="mt-3 rounded-lg border border-white/[0.07] bg-black/30 p-3 text-xs leading-5"
      data-fetch-check={check.ok ? "ok" : "failed"}
    >
      <p
        className={`flex items-center gap-1.5 font-medium ${check.ok ? "text-quality-high" : "text-red-300"}`}
      >
        {check.ok ? (
          <CheckCircle2 className="h-3.5 w-3.5" />
        ) : (
          <XCircle className="h-3.5 w-3.5" />
        )}
        {check.ok
          ? `Connected${check.me ? ` as ${check.me}` : ""}`
          : "Not connected"}
        <span className="font-normal text-zinc-600">· {time}</span>
      </p>
      <p className="mt-1 text-zinc-400">
        Health {check.health ? "OK" : "unreachable"} · Token{" "}
        {check.authorized ? "accepted" : "not accepted"}
        {check.free_space ? ` · ${check.free_space} free` : ""}
      </p>
      {check.openapi && (
        <p className="text-zinc-400">
          {check.openapi.title ?? "FETCH"}{" "}
          {check.openapi.version ? `v${check.openapi.version}` : ""} ·{" "}
          {check.openapi.path_count} API paths
          {check.openapi.missing.length === 0
            ? " · every path Mujin Music uses is there"
            : ""}
        </p>
      )}
      {check.error && (
        <p className="mt-1 flex items-start gap-1.5 text-amber-300/90">
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          {check.error}
        </p>
      )}
    </div>
  );
}

function EnhancerResult({ check }: { check: EnhancerCheck }) {
  const info = check.info;
  return (
    <div
      className="mt-3 rounded-lg border border-white/[0.07] bg-black/30 p-3 text-xs leading-5"
      data-enhancer-check={check.ok ? "ok" : "failed"}
    >
      <p
        className={`flex items-center gap-1.5 font-medium ${check.ok ? "text-quality-enhanced" : "text-red-300"}`}
      >
        {check.ok ? (
          <CheckCircle2 className="h-3.5 w-3.5" />
        ) : (
          <XCircle className="h-3.5 w-3.5" />
        )}
        {check.ok && info
          ? `${info.model.charAt(0).toUpperCase()}${info.model.slice(1)} ${info.model_version} on ${info.device_name || info.device}`
          : "Not connected"}
      </p>
      {info && (
        <p className="mt-1 text-zinc-400">
          {info.realtime_factor !== null
            ? `${info.realtime_factor.toFixed(2)} s per second of audio · `
            : ""}
          {check.realtime
            ? "fast enough for real time"
            : "not fast enough for real time (needs under 0.5)"}{" "}
          · {info.license}
        </p>
      )}
      {check.error && (
        <p className="mt-1 flex items-start gap-1.5 text-amber-300/90">
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          {check.error}
        </p>
      )}
    </div>
  );
}

function SettingsForm({ view }: { view: UpgradeSettingsView }) {
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState<UpgradeSettings>(view.settings);
  const [saving, setSaving] = useState(false);
  const [checking, setChecking] = useState<"fetch" | "enhancer" | null>(null);
  const [fetchCheck, setFetchCheck] = useState(view.fetch_check);
  const [enhancerCheck, setEnhancerCheck] = useState(view.enhancer_check);
  const saved = new Set(view.saved);
  const dirty = JSON.stringify(draft) !== JSON.stringify(view.settings);
  const refresh = async () => {
    await queryClient.invalidateQueries({ queryKey: settingsKey });
  };
  const fetch = (patch: Partial<UpgradeSettings["fetch"]>) =>
    setDraft((current) => ({
      ...current,
      fetch: { ...current.fetch, ...patch },
    }));
  const enhancer = (patch: Partial<UpgradeSettings["enhancer"]>) =>
    setDraft((current) => ({
      ...current,
      enhancer: { ...current.enhancer, ...patch },
    }));
  const save = async () => {
    setSaving(true);
    try {
      const next = await saveUpgradeSettings(draft);
      queryClient.setQueryData(settingsKey, next);
      setDraft(next.settings);
      await queryClient.invalidateQueries({ queryKey: ["library-tools"] });
      toast.success("Better versions settings saved");
    } catch (error) {
      toast(getApiErrorMessage(error, "The settings could not be saved."));
    } finally {
      setSaving(false);
    }
  };
  const check = async (which: "fetch" | "enhancer") => {
    setChecking(which);
    try {
      if (which === "fetch") setFetchCheck(await checkFetchConnection());
      else setEnhancerCheck(await checkEnhancerConnection());
    } catch (error) {
      toast(getApiErrorMessage(error, "The check could not run."));
    } finally {
      setChecking(null);
    }
  };

  return (
    <div className="max-w-2xl space-y-10">
      <section aria-labelledby="fetch-settings-title">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2
              className="text-base font-semibold text-white"
              id="fetch-settings-title"
            >
              FETCH
            </h2>
            <p className="mt-1 max-w-lg text-xs leading-5 text-zinc-500">
              Your download manager on the home network. Mujin Music's server
              asks it for lossless copies; your browser never talks to it.
            </p>
          </div>
          <RadioSwitch
            checked={draft.fetch.enabled}
            label="Use FETCH"
            onChange={(enabled) => fetch({ enabled })}
          />
        </div>
        <div className="mt-2 divide-y divide-white/8">
          <Row
            description="On your network, for example http://192.168.1.58:8490."
            label="Address"
          >
            <Input
              aria-label="FETCH address"
              onChange={(event) => fetch({ base_url: event.target.value })}
              value={draft.fetch.base_url}
            />
          </Row>
          <Row
            description={
              <p className="flex items-start gap-1.5 text-amber-300/90">
                <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                FETCH allows one agent token per app; ask FETCH for a separate
                token for Mujin Music rather than generating a new one, which
                would sign out other apps such as the voice assistant.
              </p>
            }
            label="Agent token"
          >
            <SecretField
              label="Agent token"
              name="fetch_token"
              onChanged={refresh}
              placeholder="Paste the token FETCH gave Mujin Music"
              saved={saved.has("fetch_token")}
            />
          </Row>
          <Row
            description="Only when you reach FETCH through its public address behind Cloudflare Access. Leave empty on the home network."
            label="Cloudflare Access service token"
          >
            <div className="space-y-2">
              <SecretField
                label="CF-Access-Client-Id"
                name="cf_access_client_id"
                onChanged={refresh}
                placeholder="CF-Access-Client-Id"
                saved={saved.has("cf_access_client_id")}
              />
              <SecretField
                label="CF-Access-Client-Secret"
                name="cf_access_client_secret"
                onChanged={refresh}
                placeholder="CF-Access-Client-Secret"
                saved={saved.has("cf_access_client_secret")}
              />
            </div>
          </Row>
          <Row
            description="Where FETCH's Music share is in Mujin Music, so finished albums are found with a quick rescan of that folder."
            label="FETCH saves music into this library folder"
          >
            <select
              aria-label="FETCH saves music into this library folder"
              className={selectClass}
              onChange={(event) =>
                fetch({ library_root: event.target.value || null })
              }
              value={draft.fetch.library_root ?? ""}
            >
              <option value="">None: rescan the whole library</option>
              {view.library_roots.map((root) => (
                <option key={root} value={root}>
                  {root}
                </option>
              ))}
            </select>
          </Row>
          <Row
            description="Also list CD-quality (16-bit/44.1 kHz) albums, in case a hi-res release exists."
            label="Hi-res candidates"
          >
            <RadioSwitch
              checked={draft.fetch.include_cd_quality}
              label="Hi-res candidates"
              onChange={(include_cd_quality) => fetch({ include_cd_quality })}
            />
          </Row>
          <Row
            description="Off by default. Mujin Music asks FETCH for one candidate at a time, at most this many a day."
            label="Request automatically"
          >
            <div className="flex items-center justify-end gap-3">
              <Input
                aria-label="Most automatic requests a day"
                className="w-20"
                disabled={!draft.fetch.auto_request}
                max={50}
                min={1}
                onChange={(event) =>
                  fetch({ auto_per_day: Number(event.target.value) || 1 })
                }
                type="number"
                value={draft.fetch.auto_per_day}
              />
              <span className="text-xs text-zinc-500">a day</span>
              <RadioSwitch
                checked={draft.fetch.auto_request}
                label="Request automatically"
                onChange={(auto_request) => fetch({ auto_request })}
              />
            </div>
          </Row>
          <Row
            description="After a verified lossless copy arrives, move the replaced lossy files into this folder. Nothing is ever deleted; off keeps them as an alternate version."
            label="Move replaced lossy files"
          >
            <div className="space-y-2">
              <div className="flex justify-end">
                <RadioSwitch
                  checked={draft.fetch.move_replaced}
                  label="Move replaced lossy files"
                  onChange={(move_replaced) => fetch({ move_replaced })}
                />
              </div>
              {draft.fetch.move_replaced && (
                <Input
                  aria-label="Replaced folder"
                  onChange={(event) =>
                    fetch({ replaced_folder: event.target.value || null })
                  }
                  placeholder="/music/Replaced"
                  value={draft.fetch.replaced_folder ?? ""}
                />
              )}
            </div>
          </Row>
        </div>
        <div className="mt-2">
          <Button
            disabled={checking !== null || dirty}
            onClick={() => void check("fetch")}
            title={dirty ? "Save first" : undefined}
            variant="secondary"
          >
            {checking === "fetch" ? (
              <Loader2 className="animate-spin" />
            ) : (
              <PlugZap />
            )}
            Check connection
          </Button>
          {fetchCheck && <CheckResult check={fetchCheck} />}
        </div>
      </section>

      <section aria-labelledby="enhancer-settings-title">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2
              className="flex items-center gap-2 text-base font-semibold text-white"
              id="enhancer-settings-title"
            >
              <Sparkles className="h-4 w-4 text-quality-enhanced" />
              AI enhancer
            </h2>
            <p className="mt-1 max-w-lg text-xs leading-5 text-zinc-500">
              The enhancer runs Apollo on your PC&apos;s GPU and restores lossy
              tracks. It generates plausible treble; it never recovers the
              original master. Its copies are labelled{" "}
              <span className="text-quality-enhanced">AI-enhanced</span>, never
              lossless, and the originals are always kept. To start it, see the
              &ldquo;Better versions and AI enhance&rdquo; guide.
            </p>
          </div>
          <RadioSwitch
            checked={draft.enhancer.enabled}
            label="Use the AI enhancer"
            onChange={(enabled) => enhancer({ enabled })}
          />
        </div>
        <div className="mt-2 divide-y divide-white/8">
          <Row
            description="The PC running the enhancer, for example http://192.168.1.58:8765."
            label="Address"
          >
            <Input
              aria-label="Enhancer address"
              onChange={(event) => enhancer({ base_url: event.target.value })}
              value={draft.enhancer.base_url}
            />
          </Row>
          <Row
            description="Shown in the enhancer's window when it starts."
            label="Shared secret"
          >
            <SecretField
              label="Shared secret"
              name="enhancer_token"
              onChanged={refresh}
              placeholder="Paste the enhancer's shared secret"
              saved={saved.has("enhancer_token")}
            />
          </Row>
          <Row
            description={`New files only, never over an original.${view.enhanced_folder ? ` Now: ${view.enhanced_folder}` : ""}`}
            label="Save AI-enhanced files in"
          >
            <Input
              aria-label="Enhanced folder"
              onChange={(event) =>
                enhancer({ output_folder: event.target.value || null })
              }
              placeholder={view.enhanced_folder ?? "/music/Mujin Enhanced"}
              value={draft.enhancer.output_folder ?? ""}
            />
          </Row>
          <Row
            description="Play the AI-enhanced copy instead of the lossy original. A real lossless copy always comes first either way."
            label="Prefer AI-enhanced versions"
          >
            <RadioSwitch
              checked={draft.enhancer.prefer_enhanced}
              label="Prefer AI-enhanced versions"
              onChange={(prefer_enhanced) => enhancer({ prefer_enhanced })}
            />
          </Row>
          <Row
            description="The enhancer sees the music at the same paths (same disk, or the share mounted alike), so files are not uploaded."
            label="The enhancer can read the music folders"
          >
            <RadioSwitch
              checked={draft.enhancer.shared_paths}
              label="The enhancer can read the music folders"
              onChange={(shared_paths) => enhancer({ shared_paths })}
            />
          </Row>
        </div>
        <div className="mt-2">
          <Button
            disabled={checking !== null || dirty}
            onClick={() => void check("enhancer")}
            title={dirty ? "Save first" : undefined}
            variant="secondary"
          >
            {checking === "enhancer" ? (
              <Loader2 className="animate-spin" />
            ) : (
              <PlugZap />
            )}
            Check connection
          </Button>
          {enhancerCheck && <EnhancerResult check={enhancerCheck} />}
        </div>
      </section>

      <div className="sticky bottom-24 flex items-center justify-end gap-3 rounded-xl border border-white/[0.07] bg-mujin-raised/95 p-3 backdrop-blur">
        <p className="mr-auto text-xs text-zinc-500">
          Tokens are saved on the server, encrypted, and never shown again.
        </p>
        <Button
          disabled={!dirty || saving}
          onClick={() => setDraft(view.settings)}
          variant="ghost"
        >
          Undo
        </Button>
        <Button disabled={!dirty || saving} onClick={() => void save()}>
          {saving && <Loader2 className="animate-spin" />}
          Save
        </Button>
      </div>
    </div>
  );
}

export default function BetterVersionsSettings() {
  const settings = useQuery({
    queryKey: settingsKey,
    queryFn: getUpgradeSettings,
  });
  if (settings.isPending) {
    return (
      <div className="flex h-40 items-center justify-center text-zinc-500">
        <Loader2 className="h-5 w-5 animate-spin" />
      </div>
    );
  }
  if (!settings.data) {
    return (
      <p className="text-sm text-zinc-500">
        {getApiErrorMessage(settings.error, "The settings could not load.")}
      </p>
    );
  }
  return (
    <SettingsForm
      key={JSON.stringify(settings.data.settings)}
      view={settings.data}
    />
  );
}
