"use client";

import { SourcePreferenceSetting } from "./album-versions";
import {
  checkStreamingConnection,
  formatLastSync,
  pollTidalSignIn,
  removeStreamingProvider,
  saveStreamingCredentials,
  setStreamingQuality,
  signInToQobuz,
  signOutOfStreamingProvider,
  startTidalSignIn,
  syncStreamingLibrary,
  type StreamQuality,
  type StreamingConnectionCheck,
  type StreamingPendingSignIn,
  type StreamingProviderId,
  type StreamingProviderStatus,
} from "@parson/music-sdk";
import { useQueryClient } from "@tanstack/react-query";
import {
  CheckCircle2,
  ExternalLink,
  FlaskConical,
  KeyRound,
  Loader2,
  RefreshCw,
  ShieldCheck,
  Stethoscope,
  XCircle,
} from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useSession } from "@/features/account/session-provider";
import { ServerCredentialsPanel } from "./server-credentials";
import { FormatBadge, ProviderMark } from "./source-badge";
import {
  deliverySummary,
  needsOwnCredentials,
  providerStatusLine,
  serviceAction,
} from "./streaming-model";
import {
  streamingErrorMessage,
  streamingProvidersKey,
  useStreamingProviders,
} from "./use-streaming";

const credentialLabels: Record<
  StreamingProviderId,
  {
    id: string;
    secret: string;
    secretOptional: boolean;
    where: string;
    server: string;
  }
> = {
  tidal: {
    id: "Client ID",
    secret: "Client secret",
    secretOptional: true,
    where:
      "Use the OAuth client ID (and secret, if your client has one) from your own TIDAL developer access.",
    server: "this server's TIDAL client",
  },
  qobuz: {
    id: "App ID",
    secret: "App secret",
    secretOptional: false,
    where:
      "Use the app ID and app secret Qobuz issued to you. Both are needed to stream.",
    server: "this server's Qobuz app",
  },
};

export default function ServicesSettings() {
  const { providers, sandbox, isPending, isError, refetch } =
    useStreamingProviders();
  const { session } = useSession();
  const isAdmin = session?.role === "admin";
  const [open, setOpen] = useState<StreamingProviderId | null>(null);
  const active = providers.find((provider) => provider.id === open) ?? null;

  return (
    <section className="max-w-2xl space-y-6">
      <div>
        <h2 className="text-lg font-semibold text-white">Services</h2>
        <p className="mt-1.5 text-sm leading-6 text-zinc-400">
          Connect TIDAL and Qobuz to search, browse and play their catalogs next
          to your own files. Albums you add sit in your Library with a source
          badge, and they are favorited on the service too.
        </p>
      </div>

      {sandbox && <SandboxNotice />}

      {isPending && (
        <p className="flex items-center gap-2 text-sm text-zinc-500">
          <Loader2 className="h-4 w-4 animate-spin" /> Loading services…
        </p>
      )}
      {isError && (
        <p className="text-sm text-zinc-400">
          Services could not be loaded.{" "}
          <button
            className="text-white underline-offset-4 hover:underline"
            onClick={() => void refetch()}
            type="button"
          >
            Try again
          </button>
        </p>
      )}

      <div className="divide-y divide-white/8 rounded-xl border border-white/8">
        {providers.map((provider) => (
          <div
            className="flex flex-wrap items-center gap-x-5 gap-y-3 px-5 py-4"
            key={provider.id}
          >
            <div className="flex min-w-36 items-center gap-3">
              <ProviderMark
                className="h-8 w-8 rounded-lg text-base"
                provider={provider.id}
              />
              <span className="text-[17px] font-semibold tracking-wide text-white">
                {provider.name}
              </span>
            </div>
            <div className="min-w-0 flex-1">
              <p
                className={`text-sm font-medium ${provider.status === "signed_in" ? "text-zinc-100" : provider.status === "expired" ? "text-amber-200" : "text-zinc-400"}`}
              >
                {providerStatusLine(provider)}
              </p>
              {provider.status === "signed_in" && (
                <p className="mt-0.5 text-xs text-zinc-500">
                  {libraryCountLabel(provider)} in library
                  {provider.plan ? ` · ${provider.plan}` : ""}
                </p>
              )}
            </div>
            <Button
              className="rounded-full px-6"
              onClick={() => setOpen(provider.id)}
              variant={
                provider.status === "signed_in" ? "secondary" : "default"
              }
            >
              {serviceAction(provider)}
            </Button>
          </div>
        ))}
      </div>

      <div className="flex gap-3 rounded-xl bg-white/3 px-4 py-3.5 text-[13px] leading-5 text-zinc-400">
        <ShieldCheck
          aria-hidden="true"
          className="mt-0.5 h-4 w-4 shrink-0 text-zinc-300"
        />
        <p>
          <span className="font-medium text-zinc-200">
            No API key ships with Parson.
          </span>{" "}
          TIDAL and Qobuz do not issue Parson one, and Parson never extracts one
          from their apps. Your server owner adds a TIDAL client and a Qobuz app
          once; after that you only sign in to your own account. You can also
          bring your own. Credentials and sign-in tokens are encrypted on this
          server, used only for your account, and never sent to your devices.
        </p>
      </div>

      <SourcePreferenceSetting />

      <p className="text-xs leading-5 text-zinc-500">
        Parson&apos;s TIDAL and Qobuz support was built and tested against
        recorded service responses and recorded hi-res media, not live accounts.
        Run &quot;Check connection&quot; after signing in: it is the first test
        against your real account, and it reports what the stream actually
        delivered.
      </p>

      {isAdmin && <ServerCredentialsPanel />}

      <Dialog
        open={Boolean(active)}
        onOpenChange={(next) => !next && setOpen(null)}
      >
        {active && (
          <DialogContent className="max-w-lg">
            <ServiceDialog
              key={active.id}
              onDone={() => setOpen(null)}
              provider={active}
            />
          </DialogContent>
        )}
      </Dialog>
    </section>
  );
}

const plural = (count: number, noun: string) =>
  `${count} ${noun}${count === 1 ? "" : "s"}`;

function libraryCountLabel(provider: StreamingProviderStatus) {
  return `${plural(provider.library.albums, "album")} · ${plural(provider.library.tracks, "track")}`;
}

export function SandboxNotice({ compact = false }: { compact?: boolean }) {
  return (
    <div className="flex gap-3 rounded-xl border border-amber-300/20 bg-amber-300/6 px-4 py-3 text-[13px] leading-5 text-amber-100">
      <FlaskConical aria-hidden="true" className="mt-0.5 h-4 w-4 shrink-0" />
      <p>
        <span className="font-semibold">Sandbox mode.</span>{" "}
        {compact
          ? "Recorded TIDAL and Qobuz responses; tracks play a 30-second preview tone."
          : "This server answers with recorded TIDAL and Qobuz responses instead of contacting the services, and tracks play a 30-second synthesized preview. Any credentials work here; no real account is used."}
      </p>
    </div>
  );
}

function ServiceDialog({
  provider,
  onDone,
}: {
  provider: StreamingProviderStatus;
  onDone: () => void;
}) {
  const [editingCredentials, setEditingCredentials] = useState(
    needsOwnCredentials(provider),
  );
  const ownCredentials =
    provider.credentials.configured &&
    !provider.credentials.uses_server_credentials;
  const step = editingCredentials
    ? "credentials"
    : provider.status === "signed_in"
      ? "account"
      : "sign-in";

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <ProviderMark
          className="h-9 w-9 rounded-lg text-lg"
          provider={provider.id}
        />
        <div>
          <DialogTitle className="text-xl font-semibold text-white">
            {provider.name}
          </DialogTitle>
          <DialogDescription className="text-sm text-zinc-500">
            {step === "credentials"
              ? "Step 1 of 2: your API credentials"
              : step === "sign-in"
                ? ownCredentials
                  ? "Step 2 of 2: sign in to your account"
                  : `Sign in to your ${provider.name} account`
                : "Account, library and streaming quality"}
          </DialogDescription>
        </div>
      </div>
      {provider.sandbox && <SandboxNotice compact />}
      {step === "credentials" && (
        <CredentialsStep
          onCancel={
            needsOwnCredentials(provider)
              ? undefined
              : () => setEditingCredentials(false)
          }
          onSaved={() => setEditingCredentials(false)}
          provider={provider}
        />
      )}
      {step === "sign-in" &&
        (provider.id === "tidal" ? (
          <TidalSignIn
            onChangeCredentials={() => setEditingCredentials(true)}
            provider={provider}
          />
        ) : (
          <QobuzSignIn
            onChangeCredentials={() => setEditingCredentials(true)}
            provider={provider}
          />
        ))}
      {step === "account" && (
        <AccountStep
          onChangeCredentials={() => setEditingCredentials(true)}
          onDone={onDone}
          provider={provider}
        />
      )}
    </div>
  );
}

function useRefreshProviders() {
  const queryClient = useQueryClient();
  return (status?: StreamingProviderStatus) => {
    if (status) {
      queryClient.setQueryData(
        streamingProvidersKey,
        (
          current:
            | { sandbox: boolean; providers: StreamingProviderStatus[] }
            | undefined,
        ) =>
          current && {
            ...current,
            providers: current.providers.map((item) =>
              item.id === status.id ? status : item,
            ),
          },
      );
    }
    return queryClient.invalidateQueries({ queryKey: ["streaming"] });
  };
}

function CredentialsStep({
  provider,
  onSaved,
  onCancel,
}: {
  provider: StreamingProviderStatus;
  onSaved: () => void;
  onCancel?: () => void;
}) {
  const labels = credentialLabels[provider.id];
  const [clientId, setClientId] = useState("");
  const [secret, setSecret] = useState("");
  const [saving, setSaving] = useState(false);
  const refresh = useRefreshProviders();

  async function save(useServer: boolean) {
    setSaving(true);
    try {
      const status = await saveStreamingCredentials(
        provider.id,
        useServer
          ? { use_server_credentials: true }
          : {
              client_id: clientId.trim(),
              client_secret: secret.trim() || null,
            },
      );
      await refresh(status);
      setClientId("");
      setSecret("");
      onSaved();
    } catch (error) {
      toast(streamingErrorMessage(error, "Could not save credentials."));
    } finally {
      setSaving(false);
    }
  }

  return (
    <form
      className="space-y-4"
      onSubmit={(event: FormEvent) => {
        event.preventDefault();
        void save(false);
      }}
    >
      <p className="text-sm leading-6 text-zinc-400">
        {provider.name} does not issue Parson credentials, so you bring your
        own. {labels.where}
      </p>
      <label className="grid gap-2 text-sm text-zinc-300">
        {labels.id}
        <Input
          autoComplete="off"
          onChange={(event) => setClientId(event.target.value)}
          placeholder={provider.credentials.client_id_hint ?? ""}
          required
          spellCheck={false}
          value={clientId}
        />
      </label>
      <label className="grid gap-2 text-sm text-zinc-300">
        <span>
          {labels.secret}
          {labels.secretOptional && (
            <span className="text-zinc-500"> (optional)</span>
          )}
        </span>
        <Input
          autoComplete="off"
          onChange={(event) => setSecret(event.target.value)}
          required={!labels.secretOptional}
          spellCheck={false}
          type="password"
          value={secret}
        />
      </label>
      <p className="flex items-start gap-2 text-xs leading-5 text-zinc-500">
        <KeyRound aria-hidden="true" className="mt-0.5 h-3.5 w-3.5 shrink-0" />
        Stored encrypted on this server for your account only. Changing
        credentials signs you out of {provider.name}.
      </p>
      <div className="flex flex-wrap items-center gap-3 pt-1">
        <Button className="rounded-full px-6" disabled={saving} type="submit">
          {saving ? "Saving…" : "Save and continue"}
        </Button>
        {provider.server_credentials_available && (
          <Button
            className="rounded-full"
            disabled={saving}
            onClick={() => void save(true)}
            type="button"
            variant="secondary"
          >
            Use {credentialLabels[provider.id].server}
          </Button>
        )}
        {onCancel && (
          <Button onClick={onCancel} type="button" variant="ghost">
            Cancel
          </Button>
        )}
      </div>
    </form>
  );
}

function CredentialSummary({
  provider,
  onChange,
}: {
  provider: StreamingProviderStatus;
  onChange: () => void;
}) {
  return (
    <p className="text-xs text-zinc-500">
      {provider.credentials.configured &&
      !provider.credentials.uses_server_credentials
        ? `Your ${credentialLabels[provider.id].id} ${provider.credentials.client_id_hint ?? ""}`
        : `Signs in through ${credentialLabels[provider.id].server}`}
      {" · "}
      <button
        className="text-zinc-300 underline-offset-4 hover:text-white hover:underline"
        onClick={onChange}
        type="button"
      >
        {provider.credentials.configured &&
        !provider.credentials.uses_server_credentials
          ? "Change"
          : "Use your own instead"}
      </button>
    </p>
  );
}

function secondsLeft(pending: StreamingPendingSignIn | null) {
  return pending
    ? Math.max(0, pending.expires_at - Math.floor(Date.now() / 1000))
    : 0;
}

function TidalSignIn({
  provider,
  onChangeCredentials,
}: {
  provider: StreamingProviderStatus;
  onChangeCredentials: () => void;
}) {
  const [pending, setPending] = useState<StreamingPendingSignIn | null>(
    provider.pending,
  );
  const [starting, setStarting] = useState(false);
  const [remaining, setRemaining] = useState(() =>
    secondsLeft(provider.pending),
  );
  const [message, setMessage] = useState(provider.status_message);
  const refresh = useRefreshProviders();
  const polling = useRef(false);

  useEffect(() => {
    if (!pending) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const tick = setInterval(() => setRemaining(secondsLeft(pending)), 1000);
    const poll = async (delay: number) => {
      timer = setTimeout(async () => {
        if (cancelled || polling.current) return;
        polling.current = true;
        try {
          const result = await pollTidalSignIn();
          if (cancelled) return;
          if (result.status === "pending") {
            void poll(result.interval * 1000);
            return;
          }
          setPending(null);
          if (result.status === "signed_in") {
            toast.success("Signed in to TIDAL");
          } else {
            setMessage(
              result.status === "denied"
                ? "Sign-in was declined on TIDAL."
                : "The code expired. Start again.",
            );
          }
          await refresh(result.provider ?? undefined);
        } catch (error) {
          if (!cancelled) {
            setPending(null);
            setMessage(streamingErrorMessage(error, "Sign-in failed."));
          }
        } finally {
          polling.current = false;
        }
      }, delay);
    };
    void poll(pending.interval * 1000);
    return () => {
      cancelled = true;
      clearTimeout(timer);
      clearInterval(tick);
    };
  }, [pending]);

  async function start() {
    setStarting(true);
    setMessage(null);
    try {
      const next = await startTidalSignIn();
      setPending(next);
      setRemaining(secondsLeft(next));
    } catch (error) {
      setMessage(streamingErrorMessage(error, "Could not start sign-in."));
    } finally {
      setStarting(false);
    }
  }

  if (pending) {
    const link = pending.verification_uri_complete ?? pending.verification_uri;
    return (
      <div className="space-y-5">
        <p className="text-sm leading-6 text-zinc-400">
          Open the TIDAL link on any device, sign in, and confirm this code.
          Parson never sees your TIDAL password.
        </p>
        <div className="rounded-xl border border-white/10 bg-white/3 px-5 py-5 text-center">
          <p className="text-xs font-medium uppercase tracking-[0.18em] text-zinc-500">
            Your code
          </p>
          <p
            aria-live="polite"
            className="mt-2 font-mono text-4xl font-semibold tracking-[0.3em] text-white"
          >
            {pending.user_code}
          </p>
          <a
            className="mt-4 inline-flex items-center gap-1.5 text-sm font-medium text-sky-300 hover:text-sky-200"
            href={link}
            rel="noreferrer"
            target="_blank"
          >
            {link.replace(/^https?:\/\//, "")}
            <ExternalLink aria-hidden="true" className="h-3.5 w-3.5" />
          </a>
        </div>
        <p className="flex items-center gap-2 text-sm text-zinc-400">
          <Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" />
          Waiting for approval… {Math.floor(remaining / 60)}:
          {String(remaining % 60).padStart(2, "0")} left
        </p>
        <Button onClick={() => setPending(null)} variant="ghost">
          Cancel
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-5">
      <p className="text-sm leading-6 text-zinc-400">
        TIDAL signs in with a one-time code: you approve Parson on TIDAL&apos;s
        own site, so your password never passes through this server.
      </p>
      {message && <p className="text-sm text-amber-200">{message}</p>}
      <div className="flex flex-wrap items-center gap-3">
        <Button
          className="rounded-full px-6"
          disabled={starting}
          onClick={() => void start()}
        >
          {starting ? "Starting…" : "Sign in with TIDAL"}
        </Button>
      </div>
      <CredentialSummary onChange={onChangeCredentials} provider={provider} />
    </div>
  );
}

function QobuzSignIn({
  provider,
  onChangeCredentials,
}: {
  provider: StreamingProviderStatus;
  onChangeCredentials: () => void;
}) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState(provider.status_message);
  const refresh = useRefreshProviders();

  async function submit(event: FormEvent) {
    event.preventDefault();
    setSaving(true);
    setMessage(null);
    try {
      const status = await signInToQobuz(username.trim(), password);
      setPassword("");
      toast.success("Signed in to Qobuz");
      await refresh(status);
    } catch (error) {
      setMessage(streamingErrorMessage(error, "Could not sign in to Qobuz."));
    } finally {
      setSaving(false);
    }
  }

  return (
    <form className="space-y-4" onSubmit={submit}>
      <label className="grid gap-2 text-sm text-zinc-300">
        Email or username
        <Input
          autoComplete="username"
          onChange={(event) => setUsername(event.target.value)}
          required
          value={username}
        />
      </label>
      <label className="grid gap-2 text-sm text-zinc-300">
        Password
        <Input
          autoComplete="current-password"
          onChange={(event) => setPassword(event.target.value)}
          required
          type="password"
          value={password}
        />
      </label>
      <p className="text-xs leading-5 text-zinc-500">
        Your password goes to Qobuz once to sign in and is never stored. Parson
        keeps only the session token, encrypted.
      </p>
      {message && <p className="text-sm text-amber-200">{message}</p>}
      <Button className="rounded-full px-6" disabled={saving} type="submit">
        {saving ? "Signing in…" : "Sign in"}
      </Button>
      <CredentialSummary onChange={onChangeCredentials} provider={provider} />
    </form>
  );
}

function Detail({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <p className="text-[11px] font-semibold uppercase tracking-[0.14em] text-zinc-500">
        {label}
      </p>
      <div className="mt-1.5 text-[15px] text-zinc-100">{children}</div>
    </div>
  );
}

function AccountStep({
  provider,
  onDone,
  onChangeCredentials,
}: {
  provider: StreamingProviderStatus;
  onDone: () => void;
  onChangeCredentials: () => void;
}) {
  const refresh = useRefreshProviders();
  const [syncing, setSyncing] = useState(false);
  const [busy, setBusy] = useState(false);

  async function sync() {
    setSyncing(true);
    try {
      const report = await syncStreamingLibrary(provider.id);
      const added = report.albums_added + report.tracks_added;
      const removed = report.albums_removed + report.tracks_removed;
      toast.success(
        added || removed
          ? `Library synced: ${added} added, ${removed} removed`
          : "Library is up to date",
      );
      await refresh();
    } catch (error) {
      toast(streamingErrorMessage(error, "Could not sync the library."));
    } finally {
      setSyncing(false);
    }
  }

  async function changeQuality(quality: StreamQuality) {
    try {
      await refresh(await setStreamingQuality(provider.id, quality));
    } catch (error) {
      toast(streamingErrorMessage(error, "Could not change quality."));
    }
  }

  async function signOut(remove: boolean) {
    setBusy(true);
    try {
      const status = remove
        ? await removeStreamingProvider(provider.id)
        : await signOutOfStreamingProvider(provider.id);
      await refresh(status);
      toast(
        remove
          ? `${provider.name} removed, with its albums and tracks`
          : `Signed out of ${provider.name}`,
      );
      if (remove) onDone();
    } catch (error) {
      toast(streamingErrorMessage(error, "Could not sign out."));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-6">
      <div className="grid gap-5 sm:grid-cols-2">
        <Detail label="Username">{provider.account_name ?? "Unknown"}</Detail>
        <Detail label="Status">
          Successfully signed in.
          {provider.plan && (
            <span className="text-zinc-500"> {provider.plan} plan</span>
          )}
        </Detail>
        <Detail label="Last library sync">
          <span className="tabular-nums">
            {formatLastSync(provider.last_library_sync)}
          </span>{" "}
          <button
            className="ml-2 inline-flex items-center gap-1.5 text-sky-300 hover:text-sky-200 disabled:opacity-60"
            disabled={syncing}
            onClick={() => void sync()}
            type="button"
          >
            <RefreshCw
              aria-hidden="true"
              className={`h-3.5 w-3.5 ${syncing ? "animate-spin" : ""}`}
            />
            {syncing ? "Syncing…" : "Sync library now"}
          </button>
        </Detail>
        <Detail label="In your library">{libraryCountLabel(provider)}</Detail>
      </div>
      <div>
        <p className="text-[11px] font-semibold uppercase tracking-[0.14em] text-zinc-500">
          Streaming quality
        </p>
        <Select
          onValueChange={(value) => void changeQuality(value as StreamQuality)}
          value={provider.quality}
        >
          <SelectTrigger
            aria-label={`${provider.name} streaming quality`}
            className="mt-2 w-full"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {provider.quality_options.map((option) => (
              <SelectItem key={option.id} value={option.id}>
                {option.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <p className="mt-2 text-xs leading-5 text-zinc-500">
          {
            provider.quality_options.find(
              (option) => option.id === provider.quality,
            )?.detail
          }
          . If you limit streaming quality under Playback, Parson asks{" "}
          {provider.name} for its lossy tier instead.
        </p>
      </div>
      <ConnectionCheckPanel provider={provider} />
      <CredentialSummary onChange={onChangeCredentials} provider={provider} />
      <div className="flex flex-wrap items-center justify-between gap-3 border-t border-white/8 pt-5">
        <div className="flex gap-2">
          <Button
            className="rounded-full px-5"
            disabled={busy}
            onClick={() => void signOut(false)}
            variant="secondary"
          >
            Log out
          </Button>
          <Button
            disabled={busy}
            onClick={() => {
              if (
                window.confirm(
                  `Remove ${provider.name}? This forgets your credentials and removes its albums and tracks from your Parson library. Your ${provider.name} favorites are not changed.`,
                )
              ) {
                void signOut(true);
              }
            }}
            variant="ghost"
          >
            Remove service
          </Button>
        </div>
        <Button className="rounded-full px-7" onClick={onDone}>
          Done
        </Button>
      </div>
    </div>
  );
}

const stepLabels: Record<
  StreamingConnectionCheck["steps"][number]["id"],
  string
> = {
  session: "Sign-in",
  favorites: "Favorites sync",
  stream: "Stream",
};

/**
 * "Check connection": proves the account works end to end, from sign-in
 * through favorites to the first bytes of a real stream at the chosen
 * quality, and shows the format that actually arrived.
 */
function ConnectionCheckPanel({
  provider,
}: {
  provider: StreamingProviderStatus;
}) {
  const [checking, setChecking] = useState(false);
  const [check, setCheck] = useState<StreamingConnectionCheck | null>(null);
  const refresh = useRefreshProviders();

  async function run() {
    setChecking(true);
    try {
      const result = await checkStreamingConnection(provider.id);
      setCheck(result);
      await refresh();
    } catch (error) {
      toast(streamingErrorMessage(error, "Could not run the check."));
    } finally {
      setChecking(false);
    }
  }

  const summary = check?.delivery ? deliverySummary(check.delivery) : null;
  return (
    <div className="rounded-xl border border-white/8 px-4 py-3.5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <p className="text-sm font-medium text-zinc-100">Check connection</p>
          <p className="text-xs leading-5 text-zinc-500">
            Signs in, syncs favorites and plays the first seconds of a favorite
            at your chosen quality.
          </p>
        </div>
        <Button
          className="rounded-full"
          disabled={checking}
          onClick={() => void run()}
          size="sm"
          variant="secondary"
        >
          {checking ? (
            <Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" />
          ) : (
            <Stethoscope aria-hidden="true" className="h-4 w-4" />
          )}
          {checking ? "Checking…" : check ? "Check again" : "Run check"}
        </Button>
      </div>
      {check && (
        <ol aria-live="polite" className="mt-3 space-y-2">
          {check.steps.map((step) => (
            <li className="flex items-start gap-2.5 text-[13px]" key={step.id}>
              {step.ok ? (
                <CheckCircle2
                  aria-label="Passed"
                  className="mt-0.5 h-4 w-4 shrink-0 text-emerald-400"
                />
              ) : (
                <XCircle
                  aria-label="Failed"
                  className="mt-0.5 h-4 w-4 shrink-0 text-rose-400"
                />
              )}
              <span className="min-w-0">
                <span className="font-medium text-zinc-200">
                  {stepLabels[step.id]}
                </span>
                <span className="block leading-5 text-zinc-400">
                  {step.detail}
                </span>
              </span>
            </li>
          ))}
          {summary && check.delivery && (
            <li className="flex flex-wrap items-center gap-2 pl-6.5 pt-1 text-[13px] text-zinc-400">
              <FormatBadge label={summary.badge} tone={summary.tone} />
              {check.track && (
                <span className="truncate">
                  {check.track.title} · {check.track.artist.name}
                </span>
              )}
              {summary.warning && (
                <span className="w-full text-amber-200">
                  {summary.warning}.
                </span>
              )}
            </li>
          )}
        </ol>
      )}
    </div>
  );
}
