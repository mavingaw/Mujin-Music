"use client";

import {
  formatLastSync,
  getStreamingServerCredentials,
  removeStreamingServerCredentials,
  saveStreamingServerCredentials,
  type StreamingProviderId,
  type StreamingServerCredential,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Server } from "lucide-react";
import { useState, type FormEvent } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ProviderMark } from "./source-badge";
import { streamingErrorMessage } from "./use-streaming";

const serverCredentialsKey = ["streaming", "server-credentials"] as const;

const fields: Record<
  StreamingProviderId,
  { id: string; secret: string; secretRequired: boolean; what: string }
> = {
  tidal: {
    id: "Client ID",
    secret: "Client secret",
    secretRequired: false,
    what: "an OAuth client that may use TIDAL's device sign-in",
  },
  qobuz: {
    id: "App ID",
    secret: "App secret",
    secretRequired: true,
    what: "a Qobuz app ID and secret",
  },
};

/** Settings > Services, for the server owner. */
export function ServerCredentialsPanel() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: serverCredentialsKey,
    queryFn: getStreamingServerCredentials,
  });
  const [editing, setEditing] = useState<StreamingProviderId | null>(null);

  const update = async (next: StreamingServerCredential[]) => {
    queryClient.setQueryData(serverCredentialsKey, next);
    await queryClient.invalidateQueries({ queryKey: ["streaming"] });
  };

  return (
    <section
      aria-labelledby="server-credentials-title"
      className="space-y-3 rounded-xl border border-white/8 px-5 py-4"
    >
      <div className="flex items-start gap-3">
        <Server
          aria-hidden="true"
          className="mt-0.5 h-4 w-4 shrink-0 text-zinc-400"
        />
        <div>
          <h3
            className="text-sm font-semibold text-white"
            id="server-credentials-title"
          >
            Server credentials
          </h3>
          <p className="mt-1 text-[0.8125rem] leading-5 text-zinc-400">
            Add them once and every listener on this server signs in with just
            their TIDAL approval or Qobuz email and password. Use credentials
            you are entitled to use; Mujin Music does not provide any.
          </p>
        </div>
      </div>
      <ul className="divide-y divide-white/6">
        {(query.data ?? []).map((credential) => (
          <li className="py-3" key={credential.provider}>
            <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
              <span className="flex min-w-28 items-center gap-2 text-sm font-medium text-zinc-100">
                <ProviderMark provider={credential.provider} />
                {credential.name}
              </span>
              <span className="min-w-0 flex-1 text-[0.8125rem] text-zinc-400">
                {describe(credential)}
              </span>
              {credential.source !== "environment" && (
                <div className="flex gap-1">
                  <Button
                    onClick={() =>
                      setEditing(
                        editing === credential.provider
                          ? null
                          : credential.provider,
                      )
                    }
                    size="sm"
                    variant="secondary"
                  >
                    {credential.source === "key_service"
                      ? "Use your own"
                      : credential.configured
                        ? "Replace"
                        : "Add"}
                  </Button>
                  {credential.configured &&
                    credential.source !== "key_service" && (
                      <RemoveButton
                        credential={credential}
                        onRemoved={update}
                      />
                    )}
                </div>
              )}
            </div>
            {editing === credential.provider && (
              <CredentialForm
                credential={credential}
                onSaved={async (next) => {
                  await update(next);
                  setEditing(null);
                }}
              />
            )}
          </li>
        ))}
      </ul>
      {query.isError && (
        <p className="text-[0.8125rem] text-zinc-500">
          Server credentials could not be loaded.
        </p>
      )}
    </section>
  );
}

function describe(credential: StreamingServerCredential) {
  if (!credential.configured) {
    return "Not set. Listeners need their own credentials.";
  }
  const id = `${fields[credential.provider].id} ${credential.client_id_hint ?? ""}`;
  if (credential.source === "environment") {
    return `${id} · set by environment variables`;
  }
  if (credential.source === "key_service") {
    return `${id} · from the Mujin Music key service. Nothing to set up; add your own only to override it.`;
  }
  return `${id}${credential.updated_at ? ` · saved ${formatLastSync(credential.updated_at)}` : ""}`;
}

function RemoveButton({
  credential,
  onRemoved,
}: {
  credential: StreamingServerCredential;
  onRemoved: (next: StreamingServerCredential[]) => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  return (
    <Button
      disabled={busy}
      onClick={async () => {
        if (
          !window.confirm(
            `Remove the server's ${credential.name} credentials? Listeners who sign in through them will need to sign in with their own.`,
          )
        ) {
          return;
        }
        setBusy(true);
        try {
          await onRemoved(
            await removeStreamingServerCredentials(credential.provider),
          );
          toast(`${credential.name} server credentials removed`);
        } catch (error) {
          toast(streamingErrorMessage(error, "Could not remove them."));
        } finally {
          setBusy(false);
        }
      }}
      size="sm"
      variant="ghost"
    >
      Remove
    </Button>
  );
}

function CredentialForm({
  credential,
  onSaved,
}: {
  credential: StreamingServerCredential;
  onSaved: (next: StreamingServerCredential[]) => Promise<void>;
}) {
  const labels = fields[credential.provider];
  const [clientId, setClientId] = useState("");
  const [secret, setSecret] = useState("");
  const [saving, setSaving] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setSaving(true);
    try {
      await onSaved(
        await saveStreamingServerCredentials(credential.provider, {
          client_id: clientId.trim(),
          client_secret: secret.trim() || null,
        }),
      );
      toast.success(`${credential.name} is ready for every listener`);
    } catch (error) {
      toast(streamingErrorMessage(error, "Could not save them."));
    } finally {
      setSaving(false);
    }
  }

  return (
    <form className="mt-3 grid gap-3 sm:grid-cols-2" onSubmit={submit}>
      <p className="text-[0.8125rem] leading-5 text-zinc-500 sm:col-span-2">
        Enter {labels.what}. Stored encrypted on this server and never shown
        again.
      </p>
      <label className="grid gap-1.5 text-[0.8125rem] text-zinc-300">
        {labels.id}
        <Input
          autoComplete="off"
          onChange={(event) => setClientId(event.target.value)}
          required
          spellCheck={false}
          value={clientId}
        />
      </label>
      <label className="grid gap-1.5 text-[0.8125rem] text-zinc-300">
        <span>
          {labels.secret}
          {!labels.secretRequired && (
            <span className="text-zinc-500"> (optional)</span>
          )}
        </span>
        <Input
          autoComplete="off"
          onChange={(event) => setSecret(event.target.value)}
          required={labels.secretRequired}
          spellCheck={false}
          type="password"
          value={secret}
        />
      </label>
      <div className="sm:col-span-2">
        <Button className="rounded-full px-5" disabled={saving} type="submit">
          {saving ? "Saving…" : "Save for all listeners"}
        </Button>
      </div>
    </form>
  );
}
