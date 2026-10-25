"use client";

import { tidalRedirectUri, TIDAL_CALLBACK_PATH } from "@parson/music-sdk";
import { ChevronRight } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";

/**
 * Exactly how a subscriber or server owner gets credentials, since neither
 * service issues any to Parson. Steps are the services' own processes;
 * what has and has not been verified is said plainly.
 */
export function CredentialsGuide() {
  const [redirect, setRedirect] = useState(TIDAL_CALLBACK_PATH);
  useEffect(() => setRedirect(tidalRedirectUri(window.location.origin)), []);

  return (
    <div className="space-y-2" id="credentials-guide">
      <h3 className="text-sm font-semibold text-zinc-200">
        How to get credentials
      </h3>
      <GuideSection title="TIDAL: register your own client (free, self-service)">
        <ol className="list-decimal space-y-1.5 pl-5">
          <li>
            Sign in at{" "}
            <a
              className="text-sky-300 hover:text-sky-200"
              href="https://developer.tidal.com/dashboard"
              rel="noreferrer"
              target="_blank"
            >
              developer.tidal.com
            </a>{" "}
            with your TIDAL account and create an app in the dashboard.
          </li>
          <li>
            Add this redirect URI to the app:{" "}
            <code className="break-all rounded bg-white/6 px-1 py-0.5 text-zinc-300">
              {redirect}
            </code>
          </li>
          <li>
            Enable the scopes Parson asks for: user.read, collection.read,
            collection.write, playlists.read, search.read and playback.
          </li>
          <li>
            Copy the client ID and client secret into TIDAL, &quot;I have my own
            client ID&quot; (just you) or Server credentials (everyone on this
            server).
          </li>
          <li>
            Choose &quot;Sign in on tidal.com instead&quot; and approve Parson.
            Then run Check connection.
          </li>
        </ol>
        <p className="mt-2 text-zinc-500">
          TIDAL decides which clients may stream full tracks. Whether a
          self-registered client gets full-length hi-res playback has not been
          verified: no TIDAL account was available while Parson&apos;s support
          was built. Check connection tells you what your account actually
          receives.
        </p>
      </GuideSection>
      <GuideSection title="Qobuz: ask Qobuz for an app ID and secret">
        <ol className="list-decimal space-y-1.5 pl-5">
          <li>
            Qobuz has no self-service keys. Write to Qobuz&apos;s API team,
            using the contact in its{" "}
            <a
              className="text-sky-300 hover:text-sky-200"
              href="https://github.com/Qobuz/api-documentation"
              rel="noreferrer"
              target="_blank"
            >
              API documentation
            </a>
            , and ask for an app ID and app secret for personal use of your
            subscription with Parson.
          </li>
          <li>
            Enter both under Qobuz, &quot;I have my own app ID&quot;, or under
            Server credentials.
          </li>
          <li>
            Sign in with your Qobuz email and password. Parson sends the
            password to Qobuz once and keeps only the session token.
          </li>
        </ol>
        <p className="mt-2 text-zinc-500">
          Whether and how quickly Qobuz grants a key is up to Qobuz. Parson
          never copies the keys built into Qobuz&apos;s own apps.
        </p>
      </GuideSection>
    </div>
  );
}

function GuideSection({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <details className="group rounded-xl border border-white/8 px-4 py-3 text-[13px] leading-5 text-zinc-400">
      <summary className="flex cursor-pointer list-none items-center gap-2 font-medium text-zinc-200">
        <ChevronRight
          aria-hidden="true"
          className="h-4 w-4 shrink-0 transition-transform group-open:rotate-90"
        />
        {title}
      </summary>
      <div className="mt-3">{children}</div>
    </details>
  );
}
