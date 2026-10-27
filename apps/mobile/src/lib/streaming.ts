import {
  streamingSignInPath,
  tidalSignInMethods,
  type StreamingAlbum,
  StreamingDelivery,
  StreamingArtistSummary,
  StreamingProviderId,
  StreamingProviderStatus,
  StreamingSearchResponse,
  StreamingTrack,
} from "@parson/music-sdk";

export type StreamingSearchRow =
  | {
      type: "service";
      key: string;
      provider: StreamingProviderId;
      name: string;
      error?: string;
      empty: boolean;
    }
  | { type: "album"; key: string; album: StreamingAlbum }
  | {
      type: "track";
      key: string;
      track: StreamingTrack;
      queue: StreamingTrack[];
    }
  | { type: "artist"; key: string; artist: StreamingArtistSummary };

/** Flattens per-service search results into list rows: header, artists, albums, tracks. */
export function streamingSearchRows(
  response: StreamingSearchResponse | undefined,
  limits = { artists: 3, albums: 6, tracks: 6 },
): StreamingSearchRow[] {
  const rows: StreamingSearchRow[] = [];
  for (const service of response?.services ?? []) {
    const results = service.results;
    const artists = results?.artists.slice(0, limits.artists) ?? [];
    const albums = results?.albums.slice(0, limits.albums) ?? [];
    const tracks = (results?.tracks.slice(0, limits.tracks) ?? []).filter(
      (track) => track.streamable,
    );
    rows.push({
      type: "service",
      key: `service-${service.provider}`,
      provider: service.provider,
      name: service.name,
      error: service.error,
      empty:
        !service.error && !artists.length && !albums.length && !tracks.length,
    });
    for (const artist of artists) {
      rows.push({ type: "artist", key: `artist-${artist.id}`, artist });
    }
    for (const album of albums) {
      rows.push({ type: "album", key: `album-${album.id}`, album });
    }
    for (const track of tracks) {
      rows.push({
        type: "track",
        key: `track-${track.id}`,
        track,
        queue: tracks,
      });
    }
  }
  return rows;
}

/** With server credentials, listeners skip straight to signing in. */
export function needsOwnCredentials(provider: StreamingProviderStatus) {
  return (
    !provider.credentials.configured && !provider.server_credentials_available
  );
}

export function serviceAction(provider: StreamingProviderStatus) {
  if (provider.status === "signed_in") return "Manage";
  return needsOwnCredentials(provider) ? "Set up" : "Sign in";
}

/** `4:01` style length. */
function clock(seconds: number) {
  const total = Math.max(0, Math.round(seconds));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

/** The player's signal line: what the stream actually delivered. */
export function deliveryText(
  delivery: StreamingDelivery,
  trackSeconds?: number | null,
): {
  label: string;
  detail: string;
  warning: string | null;
} {
  const format = delivery.delivered;
  if (!format) {
    return { label: "Format unknown", detail: "", warning: null };
  }
  const channels =
    format.channels === 1 ? "mono" : format.channels === 2 ? "stereo" : null;
  // Servers before `tone` existed only had the placeholder tone.
  const placeholder = delivery.tone ?? delivery.sandbox;
  const delivered = delivery.delivered_seconds ?? null;
  const track = delivery.track_seconds ?? trackSeconds ?? null;
  // The sandbox renders 30 seconds of every track, so no verdict there.
  const fullLength = delivery.sandbox
    ? null
    : typeof delivery.full_length === "boolean"
      ? delivery.full_length
      : delivered && track && track > 0
        ? delivered + 2 >= track
        : null;
  const detail = [
    placeholder
      ? "sandbox preview"
      : delivery.sandbox
        ? "sandbox, synthesized audio"
        : "delivered",
    channels,
    delivery.frame_verified ? "frame verified" : null,
    fullLength && delivered ? `full length ${clock(delivered)}` : null,
  ]
    .filter(Boolean)
    .join(" · ");
  const mismatch =
    !placeholder && !delivery.matches_announced
      ? `Announced ${delivery.announced.label}${
          delivery.mismatches?.length
            ? ` (${delivery.mismatches.join(", ")})`
            : ""
        }`
      : null;
  const preview =
    fullLength === false && delivered && track
      ? `Preview: ${clock(delivered)} of ${clock(track)}`
      : null;
  return {
    label: format.label,
    detail,
    warning: [mismatch, preview].filter(Boolean).join(" · ") || null,
  };
}

/** Seconds until a TIDAL device code expires, never negative. */
export function codeSecondsLeft(expiresAt: number, nowMs = Date.now()) {
  return Math.max(0, expiresAt - Math.floor(nowMs / 1000));
}

/** A review or biography for a phone screen: the first paragraph until expanded. */
export function editorialPreview(
  text: string,
  expanded: boolean,
): { paragraphs: string[]; more: boolean } {
  const paragraphs = text
    .split(/\n{2,}/)
    .map((paragraph) => paragraph.trim())
    .filter(Boolean);
  const more = paragraphs.length > 1 || text.length > 400;
  if (expanded || !more) return { paragraphs, more };
  const first = paragraphs[0] ?? "";
  return {
    paragraphs: [
      first.length > 400 ? `${first.slice(0, 397).trimEnd()}…` : first,
    ],
    more,
  };
}

/**
 * What a subscriber reads when the server has no TIDAL client or Qobuz app:
 * the server owner adds one once, then everyone signs in with only their
 * account.
 */
export function subscriberSetupText(
  provider: StreamingProviderId,
  name: string,
): string {
  const step =
    provider === "tidal"
      ? "approve the sign-in on TIDAL"
      : "enter your Qobuz email and password";
  const credential = provider === "tidal" ? "a TIDAL client" : "a Qobuz app";
  return `With a ${name} subscription you only need to ${step}, once this server has ${credential}. It has none yet: ask the person who runs this Mujin Music server to add one in Settings, Services, Server credentials on the web. ${name} does not issue credentials to Mujin Music, and Mujin Music never copies keys out of ${name}'s own apps.`;
}

/** The line under a service's sign-in saying whose app it signs in through. */
export function signInThroughText(provider: StreamingProviderStatus): string {
  const path = streamingSignInPath(provider);
  switch (path.kind) {
    case "own_credentials":
      return `Your credentials ${provider.credentials.client_id_hint ?? ""} · Change`;
    case "subscription":
      return path.via === "key_service"
        ? `Signs in through ${path.issuer}'s licensed ${provider.name} app · Use your own`
        : "Signs in through this server's credentials · Use your own";
    default:
      return "Use your own credentials";
  }
}

/**
 * Whether the phone can start this TIDAL sign-in itself. A key service's
 * client that TIDAL allows no device codes signs in through tidal.com in a
 * browser signed in to Parson, so the phone points there instead.
 */
export function tidalPhoneSignIn(provider: StreamingProviderStatus): {
  canStart: boolean;
  note: string | null;
} {
  if (tidalSignInMethods(provider).primary === "browser") {
    return {
      canStart: false,
      note: "TIDAL signs in on tidal.com for this server. Open Mujin Music on the web or desktop, go to Settings, Services, TIDAL and choose Sign in with TIDAL. You only log in with your TIDAL account; afterwards this phone plays TIDAL too.",
    };
  }
  return { canStart: true, note: null };
}

/** The Services footnote: how listeners on this server sign in, and why. */
export function keysNoticeText(providers: StreamingProviderStatus[]): string {
  const viaService = providers.filter((provider) => {
    const path = streamingSignInPath(provider);
    return path.kind === "subscription" && path.via === "key_service";
  });
  if (viaService.length) {
    const issuer =
      viaService[0]?.key_service?.issuer ?? "Mujin Music's key service";
    return `Sign in to ${viaService.map((provider) => provider.name).join(" and ")} with just your subscription: this server uses ${issuer}'s key service, which holds the licensed apps. It signs stream requests and relays TIDAL's sign-in, and never sees your password or session. Tokens are encrypted on your server and never stored on this phone.`;
  }
  const failing = providers.find(
    (provider) => provider.key_service?.error,
  )?.key_service;
  if (failing) {
    return `Mujin Music's key service (${failing.url}) is not answering, so your server owner's own apps are needed until it does. Tokens are encrypted on your server and never stored on this phone.`;
  }
  return "This server has no key service. TIDAL and Qobuz license their APIs to apps, and neither has issued keys to Mujin Music yet; Mujin Music never extracts keys from their apps. Your server owner adds them once, then you only sign in. Tokens are encrypted on your server and never stored on this phone.";
}
