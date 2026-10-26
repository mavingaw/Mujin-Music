"use client";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  configureImportService,
  disconnectImportService,
  getImportServices,
  importErrorMessage,
  startSpotifyAuthorization,
  type ImportService,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ChevronDown,
  ExternalLink,
  Loader2,
  ShieldCheck,
  Unplug,
} from "lucide-react";
import { useState, type FormEvent, type ReactNode } from "react";
import { toast } from "sonner";
import CapabilityChips from "./capability-chips";
import {
  SERVICE_ACCENTS,
  spotifyRedirectAllowed,
  spotifyRedirectUri,
} from "./import-review-state";

export const IMPORT_SERVICES_QUERY_KEY = ["import-services"] as const;

export function useImportServices() {
  return useQuery({
    queryKey: IMPORT_SERVICES_QUERY_KEY,
    queryFn: getImportServices,
    staleTime: 60_000,
  });
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: ReactNode;
  children: ReactNode;
}) {
  return (
    <label className="grid gap-1.5 text-xs font-medium text-zinc-400">
      {label}
      {children}
      {hint && <span className="font-normal text-zinc-600">{hint}</span>}
    </label>
  );
}

function SpotifyForm() {
  const [clientId, setClientId] = useState("");
  const [busy, setBusy] = useState(false);
  const origin = typeof window === "undefined" ? "" : window.location.origin;
  const redirectUri = spotifyRedirectUri(origin);
  const allowed = spotifyRedirectAllowed(origin);
  const loopback = origin.replace(
    /^http:\/\/(localhost|[^/:]+)(:\d+)?/,
    (_, _host, port: string | undefined) => `http://127.0.0.1${port ?? ""}`,
  );

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    try {
      const { authorize_url } = await startSpotifyAuthorization(
        clientId,
        redirectUri,
      );
      window.location.assign(authorize_url);
    } catch (error) {
      toast.error(importErrorMessage(error, "Spotify sign-in did not start."));
      setBusy(false);
    }
  }

  return (
    <form className="grid gap-3" onSubmit={(event) => void submit(event)}>
      <ol className="list-decimal space-y-1 pl-4 text-xs leading-5 text-zinc-500">
        <li>
          Create an app at{" "}
          <a
            className="text-zinc-300 underline-offset-2 hover:underline"
            href="https://developer.spotify.com/dashboard"
            rel="noreferrer"
            target="_blank"
          >
            developer.spotify.com
          </a>{" "}
          (Web API).
        </li>
        <li>
          Add this Redirect URI:{" "}
          <code className="select-all rounded bg-white/6 px-1 py-0.5 text-zinc-300">
            {redirectUri}
          </code>
        </li>
        <li>Paste the app&apos;s Client ID below. No secret is needed.</li>
      </ol>
      {!allowed && (
        <p className="rounded-md border border-amber-400/20 bg-amber-400/8 px-3 py-2 text-xs leading-5 text-amber-200">
          Spotify only redirects to https or http://127.0.0.1. Open Parson at{" "}
          <a className="underline" href={`${loopback}/settings`}>
            {loopback}
          </a>{" "}
          on this computer, or over https, to connect.
        </p>
      )}
      <Field label="Client ID">
        <Input
          autoComplete="off"
          inputMode="text"
          onChange={(event) => setClientId(event.target.value)}
          pattern="[0-9a-fA-F]{32}"
          placeholder="32 hexadecimal characters"
          required
          spellCheck={false}
          value={clientId}
        />
      </Field>
      <div>
        <Button disabled={busy || !allowed} type="submit">
          {busy && <Loader2 className="animate-spin" />}
          Continue to Spotify
        </Button>
      </div>
    </form>
  );
}

function AppleForm({ onDone }: { onDone: () => void }) {
  const [developerToken, setDeveloperToken] = useState("");
  const [userToken, setUserToken] = useState("");
  const [storefront, setStorefront] = useState("us");
  const [busy, setBusy] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    try {
      await configureImportService("apple_music", {
        developer_token: developerToken.trim(),
        music_user_token: userToken.trim() || undefined,
        storefront: storefront.trim().toLowerCase(),
      });
      toast.success("Apple Music connected.");
      onDone();
    } catch (error) {
      toast.error(importErrorMessage(error, "Apple Music was not connected."));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="grid gap-3" onSubmit={(event) => void submit(event)}>
      <Field
        hint="A JWT signed with your MusicKit private key (Apple Developer Program)."
        label="Developer token"
      >
        <Input
          autoComplete="off"
          onChange={(event) => setDeveloperToken(event.target.value)}
          placeholder="eyJhbGciOiJFUzI1NiIs…"
          required
          spellCheck={false}
          value={developerToken}
        />
      </Field>
      <Field
        hint="Optional. Needed for your library playlists; get it from MusicKit JS authorize(). Without it, public catalog playlist links still import."
        label="Music User Token"
      >
        <Input
          autoComplete="off"
          onChange={(event) => setUserToken(event.target.value)}
          spellCheck={false}
          value={userToken}
        />
      </Field>
      <Field label="Storefront">
        <Input
          className="w-24"
          maxLength={2}
          onChange={(event) => setStorefront(event.target.value)}
          pattern="[a-zA-Z]{2}"
          required
          value={storefront}
        />
      </Field>
      <div>
        <Button disabled={busy} type="submit">
          {busy && <Loader2 className="animate-spin" />}
          Verify and connect
        </Button>
      </div>
    </form>
  );
}

function YoutubeForm({ onDone }: { onDone: () => void }) {
  const [apiKey, setApiKey] = useState("");
  const [busy, setBusy] = useState(false);
  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    try {
      await configureImportService("youtube_music", { api_key: apiKey.trim() });
      toast.success("YouTube Music connected.");
      onDone();
    } catch (error) {
      toast.error(importErrorMessage(error, "The API key was not saved."));
    } finally {
      setBusy(false);
    }
  }
  return (
    <form className="grid gap-3" onSubmit={(event) => void submit(event)}>
      <Field
        hint="Enable “YouTube Data API v3” in Google Cloud and create an API key. Only public and unlisted playlists can be read."
        label="YouTube Data API key"
      >
        <Input
          autoComplete="off"
          onChange={(event) => setApiKey(event.target.value)}
          placeholder="AIza…"
          required
          spellCheck={false}
          value={apiKey}
        />
      </Field>
      <div>
        <Button disabled={busy} type="submit">
          {busy && <Loader2 className="animate-spin" />}
          Verify and save
        </Button>
      </div>
    </form>
  );
}

function DeezerForm({ onDone }: { onDone: () => void }) {
  const [profile, setProfile] = useState("");
  const [busy, setBusy] = useState(false);
  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    const profileId = /(\d+)\/?$/.exec(profile.trim())?.[1] ?? "";
    try {
      await configureImportService("deezer", { profile_id: profileId });
      toast.success(
        profileId ? "Deezer profile linked." : "Deezer public import enabled.",
      );
      onDone();
    } catch (error) {
      toast.error(importErrorMessage(error, "Deezer was not linked."));
    } finally {
      setBusy(false);
    }
  }
  return (
    <form className="grid gap-3" onSubmit={(event) => void submit(event)}>
      <Field
        hint="Optional. Your public profile link (deezer.com/profile/123…) lists your public playlists here. Playlist links work without it."
        label="Deezer profile"
      >
        <Input
          onChange={(event) => setProfile(event.target.value)}
          placeholder="https://www.deezer.com/profile/123456"
          value={profile}
        />
      </Field>
      <div>
        <Button disabled={busy} type="submit">
          {busy && <Loader2 className="animate-spin" />}
          Save
        </Button>
      </div>
    </form>
  );
}

function connectionSummary(service: ImportService): string | null {
  const connection = service.connection;
  if (!connection) return null;
  const details = connection.details;
  const parts = [connection.account_label].filter(Boolean) as string[];
  if (service.id === "spotify" && details.premium === false) {
    parts.push("Free plan: Connect handoff needs Premium");
  }
  if (service.id === "apple_music" && details.has_user_token === false) {
    parts.push("No Music User Token: catalog links only");
  }
  return parts.join(" · ") || "Connected";
}

export function ServiceCard({
  service,
  onChanged,
}: {
  service: ImportService;
  onChanged: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const connected = Boolean(service.connection);
  const summary = connectionSummary(service);

  async function disconnect() {
    if (service.id === "file") return;
    setBusy(true);
    try {
      await disconnectImportService(service.id);
      toast.success(`${service.name} disconnected. Its tokens were deleted.`);
      onChanged();
    } catch (error) {
      toast.error(importErrorMessage(error, "Could not disconnect."));
    } finally {
      setBusy(false);
    }
  }

  const done = () => {
    setOpen(false);
    onChanged();
  };

  return (
    <article className="rounded-xl border border-white/8 bg-white/2.5 p-4">
      <div className="flex flex-wrap items-start gap-3">
        <span
          aria-hidden
          className={`mt-1 h-2.5 w-2.5 shrink-0 rounded-full ${SERVICE_ACCENTS[service.id]}`}
        />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-baseline gap-x-2">
            <h3 className="text-sm font-semibold text-white">{service.name}</h3>
            {connected ? (
              <span className="text-xs text-emerald-300">{summary}</span>
            ) : service.connect_method === "upload" ||
              service.connect_method === "public" ? (
              <span className="text-xs text-zinc-500">No sign-in needed</span>
            ) : (
              <span className="text-xs text-zinc-600">Not connected</span>
            )}
          </div>
          <p className="mt-1 text-xs leading-5 text-zinc-500">
            {service.summary}
          </p>
          <div className="mt-2.5">
            <CapabilityChips capabilities={service.capabilities} />
          </div>
        </div>
        {service.id !== "file" && (
          <div className="flex shrink-0 items-center gap-1">
            {connected && (
              <Button
                disabled={busy}
                onClick={() => void disconnect()}
                size="sm"
                variant="ghost"
              >
                <Unplug />
                Disconnect
              </Button>
            )}
            <Button
              aria-expanded={open}
              onClick={() => setOpen((value) => !value)}
              size="sm"
              variant={connected ? "ghost" : "outline"}
            >
              {connected ? "Change" : "Connect"}
              <ChevronDown
                className={`transition-transform ${open ? "rotate-180" : ""}`}
              />
            </Button>
          </div>
        )}
      </div>
      {open && (
        <div className="mt-4 grid gap-3 border-t border-white/6 pt-4">
          <p className="flex items-start gap-2 text-xs leading-5 text-zinc-500">
            <ShieldCheck className="mt-0.5 h-3.5 w-3.5 shrink-0 text-zinc-400" />
            <span>
              {service.requirements} Credentials stay on this Parson server, are
              never shown again and are left out of backups.
              {service.terms_url && (
                <>
                  {" "}
                  <a
                    className="inline-flex items-center gap-0.5 text-zinc-300 underline-offset-2 hover:underline"
                    href={service.terms_url}
                    rel="noreferrer"
                    target="_blank"
                  >
                    Developer terms <ExternalLink className="h-3 w-3" />
                  </a>
                </>
              )}
            </span>
          </p>
          {service.id === "spotify" && <SpotifyForm />}
          {service.id === "apple_music" && <AppleForm onDone={done} />}
          {service.id === "youtube_music" && <YoutubeForm onDone={done} />}
          {service.id === "deezer" && <DeezerForm onDone={done} />}
        </div>
      )}
    </article>
  );
}

export default function ServiceConnections({
  showFile = false,
}: {
  showFile?: boolean;
}) {
  const queryClient = useQueryClient();
  const services = useImportServices();
  const refresh = () =>
    void queryClient.invalidateQueries({ queryKey: IMPORT_SERVICES_QUERY_KEY });

  if (services.isPending) {
    return (
      <div className="flex items-center gap-2 text-sm text-zinc-500">
        <Loader2 className="h-4 w-4 animate-spin" /> Loading services…
      </div>
    );
  }
  if (services.isError) {
    return (
      <p className="text-sm text-zinc-500">
        Music services could not be loaded.{" "}
        <button
          className="text-zinc-300 underline"
          onClick={() => void services.refetch()}
          type="button"
        >
          Try again
        </button>
      </p>
    );
  }
  return (
    <div className="grid gap-3">
      {services.data
        .filter((service) => showFile || service.id !== "file")
        .map((service) => (
          <ServiceCard key={service.id} onChanged={refresh} service={service} />
        ))}
    </div>
  );
}
