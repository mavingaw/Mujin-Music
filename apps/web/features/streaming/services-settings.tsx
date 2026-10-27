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
  startTidalBrowserSignIn,
  startTidalSignIn,
  streamingSignInPath,
  syncStreamingLibrary,
  tidalRedirectUri,
  tidalSignInMethods,
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
import { AccountProofChip } from "./account-proof";
import { CredentialsGuide } from "./credentials-guide";
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
    /** Where an owner gets this credential, said plainly. */
    source: string;
    /** What a listener does once the server has credentials. */
    account: string;
  }
> = {
  tidal: {
    id: "Client ID",
    secret: "Client secret",
    secretOptional: true,
    where:
      "Use the OAuth client ID (and secret, if your client has one) from your own TIDAL developer access.",
    server: "this server's TIDAL client",
    source:
      "Anyone with a TIDAL account can register a client at developer.tidal.com; see How to get credentials on the Services page. Device-code sign-in needs a client TIDAL allows to use it; a self-registered client signs in on tidal.com.",
    account: "approve the sign-in on TIDAL",
  },
  qobuz: {
    id: "App ID",
    secret: "App secret",
    secretOptional: false,
    where:
      "Use the app ID and app secret Qobuz issued to you. Both are needed to stream.",
    server: "this server's Qobuz app",
    source:
      "Qobuz issues app IDs and secrets on request; see How to get credentials on the Services page.",
    account: "enter their Qobuz email and password",
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
            className="flex items-center gap-3.5 px-4 py-4 sm:gap-4 sm:px-5"
            key={provider.id}
          >
            <ProviderMark
              className="h-9 w-9 shrink-0 rounded-lg text-base"
              provider={provider.id}
            />
            {/* Name above status: the status gets the full width on a phone
                instead of a squeezed middle column. */}
            <div className="min-w-0 flex-1">
              <p className="text-[17px] font-semibold tracking-wide text-white">
                {provider.name}
              </p>
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
              {provider.status === "signed_in" && (
                <div className="mt-2">
                  <AccountProofChip provider={provider} />
                </div>
              )}
            </div>
            <Button
              className="shrink-0 rounded-full px-5 sm:px-6"
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

      <KeysNotice providers={providers} />

      <CredentialsGuide />

      <SourcePreferenceSetting />

      <div className="rounded-xl border border-dashed border-white/10 px-4 py-3.5 text-xs leading-5 text-zinc-500">
        <p className="font-medium text-zinc-300">What has been tested</p>
        <p className="mt-1">
          Mujin Music&apos;s TIDAL and Qobuz support was built and tested
          against recorded service responses and hi-res media rendered to match
          them (24/192 FLAC, TIDAL Max DASH, CD FLAC, AAC, MP3). It has not yet
          been run against a live TIDAL or Qobuz account, so full-length hi-res
          from the real services is unproven until your own check passes. Run
          Check connection after signing in: an account shows{" "}
          <span className="text-emerald-300">Verified</span> only once the real
          service has sent it a whole track, lossless, in the announced format.
        </p>
        <p className="mt-2">
          Subscription-only sign-in through a Mujin Music key service (Qobuz
          signing, the TIDAL relay) was tested by running the key service&apos;s
          own code against the same recorded replies. No key service holding
          real TIDAL or Qobuz licences exists yet.
        </p>
      </div>

      {isAdmin && <ServerCredentialsPanel />}

      <Dialog
        open={Boolean(active)}
        onOpenChange={(next) => !next && setOpen(null)}
      >
        {active && (
          <DialogContent className="max-w-lg">
            <ServiceDialog
              isAdmin={isAdmin}
              key={active.id}
              onDone={() => setOpen(null)}
              onOpenServerCredentials={() => {
                setOpen(null);
                window.setTimeout(
                  () =>
                    document
                      .getElementById("server-credentials-title")
                      ?.scrollIntoView({ behavior: "smooth", block: "center" }),
                  50,
                );
              }}
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

/**
 * Says how listeners sign in on this server, and why: with a Parson key
 * service a subscription is all anyone needs; without one, somebody adds
 * an app once. Never claims more than the server reports.
 */
function KeysNotice({ providers }: { providers: StreamingProviderStatus[] }) {
  const viaService = providers.filter((provider) => {
    const path = streamingSignInPath(provider);
    return path.kind === "subscription" && path.via === "key_service";
  });
  const service = providers.find(
    (provider) => provider.key_service,
  )?.key_service;
  if (viaService.length) {
    const issuer = service?.issuer ?? "Mujin Music";
    const names = viaService.map((provider) => provider.name).join(" and ");
    return (
      <div className="flex gap-3 rounded-xl border border-emerald-300/15 bg-emerald-300/5 px-4 py-3.5 text-[13px] leading-5 text-zinc-300">
        <ShieldCheck
          aria-hidden="true"
          className="mt-0.5 h-4 w-4 shrink-0 text-emerald-300"
        />
        <p>
          <span className="font-medium text-white">
            Sign in to {names} with just your subscription.
          </span>{" "}
          This server uses {issuer}&apos;s key service, which holds the licensed
          TIDAL client and Qobuz app, so nobody registers anything. You log in
          to TIDAL on tidal.com; your Qobuz password goes to Qobuz only. The key
          service signs stream requests and relays TIDAL&apos;s sign-in back
          here, and never sees your password or session. Sign-in tokens are
          encrypted on this server for your account only.
        </p>
      </div>
    );
  }
  return (
    <div className="flex gap-3 rounded-xl bg-white/3 px-4 py-3.5 text-[13px] leading-5 text-zinc-400">
      <ShieldCheck
        aria-hidden="true"
        className="mt-0.5 h-4 w-4 shrink-0 text-zinc-300"
      />
      <p>
        <span className="font-medium text-zinc-200">
          {service
            ? "Mujin Music's key service is not answering."
            : "This server has no key service."}
        </span>{" "}
        {service?.error
          ? `${service.url} could not be read (${service.error}). Until it answers, `
          : "TIDAL and Qobuz license their APIs to apps, not to listeners. A Mujin Music key service holding those licences lets a subscriber sign in with nothing else, but this build names none: neither service has issued keys to Mujin Music yet, and Mujin Music never extracts keys from their apps. So "}
        your server owner adds a TIDAL client and a Qobuz app once; after that
        you only sign in to your own account. You can also bring your own.
        Credentials and sign-in tokens are encrypted on this server, used only
        for your account, and never sent to your devices.
      </p>
    </div>
  );
}

export function SandboxNotice({ compact = false }: { compact?: boolean }) {
  return (
    <div className="flex gap-3 rounded-xl border border-amber-300/20 bg-amber-300/6 px-4 py-3 text-[13px] leading-5 text-amber-100">
      <FlaskConical aria-hidden="true" className="mt-0.5 h-4 w-4 shrink-0" />
      <p>
        <span className="font-semibold">Sandbox mode.</span>{" "}
        {compact
          ? "Recorded TIDAL and Qobuz responses; tracks play a 30-second synthesized tone in the format each reply announces."
          : "This server answers with recorded TIDAL and Qobuz responses instead of contacting the services. Tracks play a 30-second synthesized tone, rendered in the format each recorded reply announces (24/192 FLAC, TIDAL Max as DASH, CD FLAC, AAC, MP3) and streamed through the same proxy and format check as a real account. Sign-in goes through a simulated Mujin Music key service, so it looks the way it does for a subscriber; any email and password work and no real account is used. Sandbox mode is off unless the server is started with PARSON_STREAMING_SANDBOX=1."}
      </p>
    </div>
  );
}

function ServiceDialog({
  provider,
  isAdmin,
  onDone,
  onOpenServerCredentials,
}: {
  provider: StreamingProviderStatus;
  isAdmin: boolean;
  onDone: () => void;
  onOpenServerCredentials: () => void;
}) {
  const [editingCredentials, setEditingCredentials] = useState(
    needsOwnCredentials(provider),
  );
  // Without server credentials, explain the subscriber path first instead
  // of opening on an API credentials form.
  const [chooseOwn, setChooseOwn] = useState(
    provider.sandbox || !needsOwnCredentials(provider),
  );
  const ownCredentials =
    provider.credentials.configured &&
    !provider.credentials.uses_server_credentials;
  const step = editingCredentials
    ? chooseOwn
      ? "credentials"
      : "setup"
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
            {step === "setup"
              ? `Connect your ${provider.name} subscription`
              : step === "credentials"
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
      {step === "setup" && (
        <SubscriberSetup
          isAdmin={isAdmin}
          onOpenServerCredentials={onOpenServerCredentials}
          onUseOwn={() => setChooseOwn(true)}
          provider={provider}
        />
      )}
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

/**
 * What a subscriber does when this server has no TIDAL client or Qobuz app:
 * the owner adds one once, then every listener signs in with only their
 * account. Bringing your own stays one click away.
 */
function SubscriberSetup({
  provider,
  isAdmin,
  onUseOwn,
  onOpenServerCredentials,
}: {
  provider: StreamingProviderStatus;
  isAdmin: boolean;
  onUseOwn: () => void;
  onOpenServerCredentials: () => void;
}) {
  const labels = credentialLabels[provider.id];
  return (
    <div className="space-y-4 text-sm leading-6 text-zinc-400">
      <p>
        {isAdmin
          ? `Add ${labels.server.replace("this server's", "a")} once under Server credentials, and everyone on this server connects by signing in: they ${labels.account}, nothing else.`
          : `With a ${provider.name} subscription you only need to ${labels.account}, once this server has ${labels.server.replace("this server's", "a")}. It has none yet: ask the person who runs this Mujin Music server to add one in Settings, Services, Server credentials.`}
      </p>
      <p className="rounded-lg border border-white/8 px-3 py-2.5 text-[13px] leading-5 text-zinc-500">
        {provider.name} does not issue credentials to Mujin Music, and Mujin
        Music never copies keys out of {provider.name}&apos;s own apps.{" "}
        {labels.source}
      </p>
      <div className="flex flex-wrap items-center gap-3 pt-1">
        {isAdmin && (
          <Button
            className="rounded-full px-6"
            onClick={onOpenServerCredentials}
          >
            Add server credentials
          </Button>
        )}
        <Button
          className="rounded-full"
          onClick={onUseOwn}
          variant={isAdmin ? "secondary" : "default"}
        >
          I have my own {labels.id.toLowerCase()}
        </Button>
      </div>
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
        {provider.name} does not issue Mujin Music credentials, so you bring
        your own. {labels.where}
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

function signsInThrough(provider: StreamingProviderStatus) {
  const path = streamingSignInPath(provider);
  if (path.kind === "own_credentials") {
    return `Your ${credentialLabels[provider.id].id} ${provider.credentials.client_id_hint ?? ""}`;
  }
  if (path.kind === "subscription" && path.via === "key_service") {
    return `Signs in through ${path.issuer}'s licensed ${provider.name} app`;
  }
  return `Signs in through ${credentialLabels[provider.id].server}`;
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
      {signsInThrough(provider)}
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

  async function startInBrowser() {
    setStarting(true);
    setMessage(null);
    try {
      const next = await startTidalBrowserSignIn(
        tidalRedirectUri(window.location.origin),
      );
      window.location.assign(next.authorize_url);
    } catch (error) {
      setMessage(streamingErrorMessage(error, "Could not start sign-in."));
      setStarting(false);
    }
  }

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
          Mujin Music never sees your TIDAL password.
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

  const methods = tidalSignInMethods(provider);
  const path = streamingSignInPath(provider);
  const viaKeyService =
    path.kind === "subscription" && path.via === "key_service";
  const run = (method: "device_code" | "browser") =>
    method === "browser" ? startInBrowser() : start();
  return (
    <div className="space-y-5">
      <p className="text-sm leading-6 text-zinc-400">
        {methods.primary === "browser"
          ? "You log in on tidal.com and approve Mujin Music, and TIDAL sends you straight back here. Your password never passes through this server."
          : "TIDAL signs in with a one-time code: you approve Mujin Music on TIDAL's own site, so your password never passes through this server."}
      </p>
      {message && <p className="text-sm text-amber-200">{message}</p>}
      <div className="flex flex-wrap items-center gap-3">
        <Button
          className="rounded-full px-6"
          disabled={starting}
          onClick={() => void run(methods.primary)}
        >
          {starting ? "Starting…" : "Sign in with TIDAL"}
        </Button>
        {methods.secondary && (
          <Button
            className="rounded-full"
            disabled={starting}
            onClick={() => void run(methods.secondary!)}
            variant="secondary"
          >
            {methods.secondary === "browser"
              ? "Sign in on tidal.com instead"
              : "Use a code instead"}
          </Button>
        )}
      </div>
      {!viaKeyService && (
        <p className="text-xs leading-5 text-zinc-500">
          The code works with clients TIDAL allows to use device sign-in. A
          client you register yourself at developer.tidal.com signs in on
          tidal.com instead; register{" "}
          <code className="break-all rounded bg-white/6 px-1 py-0.5 text-zinc-300">
            {typeof window === "undefined"
              ? "/settings/services/tidal-callback"
              : tidalRedirectUri(window.location.origin)}
          </code>{" "}
          as its redirect URI.
        </p>
      )}
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
        Your password goes to Qobuz once to sign in and is never stored. Mujin
        Music keeps only the session token, encrypted.
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
          . If you limit streaming quality under Playback, Mujin Music asks{" "}
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
                  `Remove ${provider.name}? This forgets your credentials and removes its albums and tracks from your Mujin Music library. Your ${provider.name} favorites are not changed.`,
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
            Signs in, syncs favorites and reads a favorite at your chosen
            quality: its format and its full length, so a preview fails.
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
          {checking
            ? "Checking…"
            : check || provider.last_check
              ? "Check again"
              : "Run check"}
        </Button>
      </div>
      <div className="mt-3">
        <AccountProofChip provider={provider} showDetail />
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
