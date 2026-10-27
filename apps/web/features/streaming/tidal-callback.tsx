"use client";

import { finishTidalBrowserSignIn, readTidalCallback } from "@parson/music-sdk";
import { useQueryClient } from "@tanstack/react-query";
import { CheckCircle2, Loader2, XCircle } from "lucide-react";
import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { ProviderMark } from "./source-badge";
import { streamingErrorMessage } from "./use-streaming";

type CallbackState =
  | { phase: "working" }
  | { phase: "done"; account: string | null }
  | { phase: "failed"; message: string };

/**
 * Where TIDAL sends a browser sign-in back. The code and state in the URL
 * go to the Parson server, which trades them for tokens; nothing is kept in
 * the browser.
 */
export default function TidalCallback() {
  const [state, setState] = useState<CallbackState>({ phase: "working" });
  const queryClient = useQueryClient();
  const started = useRef(false);

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    const answer = readTidalCallback(window.location.search);
    // Keep the one-time code out of the address bar and history.
    window.history.replaceState(null, "", window.location.pathname);
    if (!answer.ok) {
      setState({ phase: "failed", message: answer.error });
      return;
    }
    finishTidalBrowserSignIn(answer.state, answer.code)
      .then(async (status) => {
        setState({ phase: "done", account: status.account_name });
        await queryClient.invalidateQueries({ queryKey: ["streaming"] });
      })
      .catch((error: unknown) =>
        setState({
          phase: "failed",
          message: streamingErrorMessage(error, "Could not finish signing in."),
        }),
      );
  }, [queryClient]);

  return (
    <div className="mx-auto flex w-full max-w-md flex-col items-center px-5 py-20 text-center">
      <ProviderMark
        className="h-12 w-12 rounded-xl text-2xl"
        provider="tidal"
      />
      {state.phase === "working" && (
        <p className="mt-6 flex items-center gap-2 text-sm text-zinc-300">
          <Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" />
          Finishing TIDAL sign-in…
        </p>
      )}
      {state.phase === "done" && (
        <>
          <h1 className="mt-6 flex items-center gap-2 text-xl font-semibold text-white">
            <CheckCircle2
              aria-hidden="true"
              className="h-5 w-5 text-emerald-300"
            />
            Signed in to TIDAL
          </h1>
          {state.account && (
            <p className="mt-2 text-sm text-zinc-400">as {state.account}</p>
          )}
        </>
      )}
      {state.phase === "failed" && (
        <>
          <h1 className="mt-6 flex items-center gap-2 text-xl font-semibold text-white">
            <XCircle aria-hidden="true" className="h-5 w-5 text-amber-300" />
            TIDAL sign-in did not finish
          </h1>
          <p className="mt-2 text-sm leading-6 text-zinc-400">
            {state.message}
          </p>
        </>
      )}
      {state.phase !== "working" && (
        <Button asChild className="mt-8 rounded-full px-6">
          <Link href="/settings?tab=services">Back to Services</Link>
        </Button>
      )}
    </div>
  );
}
