"use client";

import { completeLastfm } from "@parson/music-sdk";
import { CheckCircle2, Loader2, XCircle } from "lucide-react";
import Link from "next/link";
import { useSearchParams } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { ServiceMark } from "./scrobbling-settings";

type State =
  | { phase: "working" }
  | { phase: "done"; account: string }
  | { phase: "failed"; message: string };

/**
 * Where Last.fm sends the browser after "Yes, allow access". Works in any
 * browser, signed in or not: the one-time state names the profile.
 */
export default function LastfmCallback() {
  const params = useSearchParams();
  const [state, setState] = useState<State>({ phase: "working" });
  const started = useRef(false);

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    const stateParam = params.get("state");
    const token = params.get("token");
    if (!stateParam || !token) {
      setState({
        phase: "failed",
        message: "Last.fm did not approve the connection.",
      });
      return;
    }
    void completeLastfm(stateParam, token)
      .then((result) => {
        setState({ phase: "done", account: result.accountName });
        try {
          window.opener?.postMessage(
            { type: "parson:lastfm-connected" },
            window.location.origin,
          );
        } catch {
          // The opener may be gone or on another origin.
        }
        if (window.opener) window.setTimeout(() => window.close(), 1600);
      })
      .catch((error: unknown) => {
        const data = (error as { response?: { data?: { message?: unknown } } })
          ?.response?.data;
        setState({
          phase: "failed",
          message:
            typeof data?.message === "string"
              ? data.message
              : "The connection could not be finished.",
        });
      });
  }, [params]);

  return (
    <main className="flex min-h-screen items-center justify-center px-6">
      <div className="w-full max-w-sm rounded-3xl border border-white/10 bg-mujin-raised p-8 text-center shadow-[0_30px_80px_-20px_rgba(0,0,0,0.9)]">
        <div className="mx-auto flex w-fit items-center gap-3">
          <ServiceMark service="lastfm" size={52} />
        </div>
        {state.phase === "working" && (
          <>
            <h1 className="mt-6 text-lg font-semibold">Connecting Last.fm…</h1>
            <Loader2 className="mx-auto mt-4 h-5 w-5 animate-spin text-zinc-400" />
          </>
        )}
        {state.phase === "done" && (
          <>
            <h1 className="mt-6 flex items-center justify-center gap-2 text-lg font-semibold">
              <CheckCircle2 className="h-5 w-5 text-emerald-400" />
              Connected
            </h1>
            <p className="mt-2 text-sm leading-6 text-zinc-400">
              This profile now scrobbles to Last.fm as{" "}
              <span className="font-medium text-white">{state.account}</span>.
              You can close this window.
            </p>
          </>
        )}
        {state.phase === "failed" && (
          <>
            <h1 className="mt-6 flex items-center justify-center gap-2 text-lg font-semibold">
              <XCircle className="h-5 w-5 text-rose-400" />
              Not connected
            </h1>
            <p className="mt-2 text-sm leading-6 text-zinc-400">
              {state.message}
            </p>
          </>
        )}
        <Link
          className="mt-6 inline-flex h-10 items-center justify-center rounded-full bg-white px-5 text-sm font-semibold text-black hover:bg-zinc-200"
          href="/settings?tab=scrobbling"
        >
          Back to Mujin Music
        </Link>
      </div>
    </main>
  );
}
