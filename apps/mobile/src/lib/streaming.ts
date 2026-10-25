import type {
  StreamingAlbum,
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

/** The player's signal line: what the stream actually delivered. */
export function deliveryText(delivery: StreamingDelivery): {
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
  const detail = [
    delivery.sandbox ? "sandbox preview" : "delivered",
    channels,
    delivery.frame_verified ? "frame verified" : null,
  ]
    .filter(Boolean)
    .join(" · ");
  return {
    label: format.label,
    detail,
    warning:
      !delivery.sandbox && !delivery.matches_announced
        ? `Announced ${delivery.announced.label}`
        : null,
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
  return `With a ${name} subscription you only need to ${step}, once this server has ${credential}. It has none yet: ask the person who runs this Parson server to add one in Settings, Services, Server credentials on the web. ${name} does not issue credentials to Parson, and Parson never copies keys out of ${name}'s own apps.`;
}
