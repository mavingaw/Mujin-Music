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
      <GuideSection title="Signing in with only your subscription">
        <p>
          TIDAL and Qobuz license their APIs to apps, not to listeners. Apps
          that need only your password are licensed partners, or they reuse the
          keys inside the services&apos; own apps. Mujin Music does not reuse
          those keys. Instead, whoever holds a licence runs a{" "}
          <span className="text-zinc-200">Mujin Music key service</span>: it
          publishes the licensed TIDAL client and Qobuz app ID, signs each Qobuz
          stream request so the Qobuz secret never leaves it, and relays
          TIDAL&apos;s sign-in back to your server. It never sees your password
          or session. Every server that uses one lets listeners sign in with
          nothing but their subscription.
        </p>
        <p className="mt-2">
          Neither service has issued Mujin Music a licence yet, so this build
          names no key service, and until one exists somebody on this server
          adds a TIDAL client and a Qobuz app once, as described below. A server
          that holds keys can serve them to others: set
          PARSON_KEY_SERVICE_PUBLIC_URL and the
          PARSON_KEY_SERVICE_TIDAL_CLIENT_ID or PARSON_KEY_SERVICE_QOBUZ_APP_ID
          and _APP_SECRET variables there, and PARSON_KEY_SERVICE_URL on each
          server that uses it.
        </p>
      </GuideSection>
      <GuideSection title="TIDAL: register your own client (free, self-service)">
        <ol className="list-decimal space-y-1.5 pl-5">
          <li>
            Sign in at{" "}
            <a
              className="text-mujin-copper hover:text-mujin-copper-hover"
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
            Enable the scopes Mujin Music asks for: user.read, collection.read,
            collection.write, playlists.read, search.read and playback.
          </li>
          <li>
            Copy the client ID and client secret into TIDAL, &quot;I have my own
            client ID&quot; (just you) or Server credentials (everyone on this
            server).
          </li>
          <li>
            Choose &quot;Sign in on tidal.com instead&quot; and approve Mujin
            Music. Then run Check connection.
          </li>
        </ol>
        <p className="mt-2 text-zinc-500">
          TIDAL decides which clients may stream full tracks. Whether a
          self-registered client gets full-length hi-res playback has not been
          verified: no TIDAL account was available while Mujin Music&apos;s
          support was built. Check connection tells you what your account
          actually receives, and fails if it is a preview: it compares the
          length the stream itself states with the track&apos;s length, and the
          format that arrives with the one TIDAL announced.
        </p>
      </GuideSection>
      <GuideSection title="Qobuz: ask Qobuz for an app ID and secret">
        <ol className="list-decimal space-y-1.5 pl-5">
          <li>
            Qobuz has no self-service keys. Write to Qobuz&apos;s API team,
            using the contact in its{" "}
            <a
              className="text-mujin-copper hover:text-mujin-copper-hover"
              href="https://github.com/Qobuz/api-documentation"
              rel="noreferrer"
              target="_blank"
            >
              API documentation
            </a>
            , and ask for an app ID and app secret for personal use of your
            subscription with Mujin Music.
          </li>
          <li>
            Enter both under Qobuz, &quot;I have my own app ID&quot;, or under
            Server credentials.
          </li>
          <li>
            Sign in with your Qobuz email and password. Mujin Music sends the
            password to Qobuz once and keeps only the session token.
          </li>
        </ol>
        <p className="mt-2 text-zinc-500">
          Whether and how quickly Qobuz grants a key is up to Qobuz. Mujin Music
          never copies the keys built into Qobuz&apos;s own apps.
        </p>
      </GuideSection>
      <GuideSection title="Prove it with your account (for server owners)">
        <p>
          Mujin Music&apos;s source includes a test that signs in with your own
          accounts, syncs favorites and streams a favorite at the highest
          quality through this proxy. It fails unless the whole track arrives in
          the announced format. Run it once on your computer:
        </p>
        <pre className="mt-2 overflow-x-auto rounded-lg bg-white/6 px-3 py-2 text-[0.75rem] leading-5 text-zinc-300">
          cargo test -p parson-music live_accounts -- --ignored --nocapture
        </pre>
        <p className="mt-2">
          with your PARSON_LIVE_TIDAL_CLIENT_ID or PARSON_LIVE_QOBUZ_* values
          set. After you approve TIDAL once it prints a refresh token; saved as
          a repository secret, it lets the Live Streaming Accounts workflow
          repeat the check every week without anyone signing in.
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
    <details className="group rounded-xl border border-white/8 px-4 py-3 text-[0.8125rem] leading-5 text-zinc-400">
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
