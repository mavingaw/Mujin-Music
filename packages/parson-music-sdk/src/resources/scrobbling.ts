import api from "../core/http";

export type ScrobbleService = "lastfm" | "listenbrainz";

export interface ScrobbleConnection {
  accountName: string;
  profileUrl: string;
  enabled: boolean;
  sendNowPlaying: boolean;
  /** Also scrobble tracks Radio picked. */
  scrobbleRadio: boolean;
  status: "ok" | "reauth_required";
  lastError: string | null;
  lastScrobbledAt: number | null;
  connectedAt: number;
}

export interface ScrobbleQueueCounts {
  pending: number;
  failed: number;
  sent: number;
  ignored: number;
  nextAttemptAt: number | null;
}

export interface ScrobbleImportStatus {
  status: "running" | "done" | "failed";
  fetched: number;
  matched: number;
  imported: number;
  error: string | null;
  startedAt: number;
  finishedAt: number | null;
}

export interface ScrobbleServiceStatus {
  service: ScrobbleService;
  name: string;
  /** False when Last.fm still needs the server's API key. */
  available: boolean;
  connection: ScrobbleConnection | null;
  queue: ScrobbleQueueCounts;
  import: ScrobbleImportStatus | null;
}

export interface ScrobbleLogEntry {
  id: number;
  service: ScrobbleService;
  songId: string | null;
  artist: string;
  track: string;
  album: string | null;
  listenedAt: number;
  status: "pending" | "sent" | "ignored" | "failed";
  attempts: number;
  lastError: string | null;
  nextAttemptAt: number;
  sentAt: number | null;
}

export interface LastfmSetup {
  configured: boolean;
  source: "settings" | "environment" | "sandbox" | null;
  keyHint: string | null;
}

export interface ScrobblingStatus {
  services: ScrobbleServiceStatus[];
  lastfmSetup: LastfmSetup;
  recent: ScrobbleLogEntry[];
  sandbox: boolean;
}

export interface ScrobbleSettings {
  enabled?: boolean;
  sendNowPlaying?: boolean;
  scrobbleRadio?: boolean;
}

export async function getScrobbling(): Promise<ScrobblingStatus> {
  return (await api.get<ScrobblingStatus>("/scrobbling")).data;
}

/**
 * Starts connecting Last.fm. Open `authorizeUrl` in a browser; Last.fm
 * returns to this server's `/connect/lastfm` page, which finishes the
 * connection. Poll {@link getScrobbling} to notice it.
 */
export async function authorizeLastfm(
  origin: string,
): Promise<{ authorizeUrl: string; expiresIn: number }> {
  return (
    await api.post<{ authorizeUrl: string; expiresIn: number }>(
      "/scrobbling/lastfm/authorize",
      { origin },
    )
  ).data;
}

/** Finishes Last.fm's web authentication (no session needed). */
export async function completeLastfm(
  state: string,
  token: string,
): Promise<{ service: "lastfm"; accountName: string }> {
  return (
    await api.post<{ service: "lastfm"; accountName: string }>(
      "/scrobbling/lastfm/complete",
      { state, token },
      { skipAuth: true },
    )
  ).data;
}

export async function connectListenbrainz(
  token: string,
): Promise<{ service: "listenbrainz"; accountName: string }> {
  return (
    await api.put<{ service: "listenbrainz"; accountName: string }>(
      "/scrobbling/listenbrainz",
      { token: token.trim() },
    )
  ).data;
}

export async function updateScrobbleSettings(
  service: ScrobbleService,
  settings: ScrobbleSettings,
): Promise<ScrobblingStatus> {
  return (await api.patch<ScrobblingStatus>(`/scrobbling/${service}`, settings))
    .data;
}

export async function disconnectScrobbling(
  service: ScrobbleService,
): Promise<void> {
  await api.delete(`/scrobbling/${service}`);
}

/** Makes waiting and failed listens due now. */
export async function retryScrobbles(): Promise<number> {
  return (await api.post<{ queued: number }>("/scrobbling/retry")).data.queued;
}

/** Imports listening history from the service into this profile. */
export async function importScrobbleHistory(
  service: ScrobbleService,
): Promise<ScrobbleImportStatus> {
  return (await api.post<ScrobbleImportStatus>(`/scrobbling/${service}/import`))
    .data;
}

/** Saves the server's Last.fm API key and secret (administrators). */
export async function setLastfmServerCredentials(
  apiKey: string,
  apiSecret: string,
): Promise<LastfmSetup> {
  return (
    await api.put<LastfmSetup>("/scrobbling/lastfm/server", {
      apiKey: apiKey.trim(),
      apiSecret: apiSecret.trim(),
    })
  ).data;
}

export async function clearLastfmServerCredentials(): Promise<LastfmSetup> {
  return (await api.delete<LastfmSetup>("/scrobbling/lastfm/server")).data;
}

/** A ListenBrainz user token is a UUID. */
export function plausibleListenbrainzToken(value: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(
    value.trim(),
  );
}

/** A Last.fm API key or shared secret: 32 hexadecimal characters. */
export function plausibleLastfmCredential(value: string): boolean {
  return /^[0-9a-f]{32}$/i.test(value.trim());
}

/** The state of one service in a sentence, for settings and badges. */
export function scrobbleServiceSummary(
  service: ScrobbleServiceStatus,
  now: number = Math.floor(Date.now() / 1000),
): { tone: "ok" | "warn" | "error" | "idle"; text: string } {
  const connection = service.connection;
  if (!connection) {
    return service.available
      ? { tone: "idle", text: "Not connected" }
      : { tone: "idle", text: "Needs the server's Last.fm API key" };
  }
  const pending = service.queue.pending;
  // Listens keep queueing while a service is paused or needs reconnecting;
  // they are sent once it is on and connected again.
  if (connection.status === "reauth_required") {
    return {
      tone: "error",
      text: pending > 0 ? `Reconnect · ${pending} waiting` : "Reconnect needed",
    };
  }
  if (!connection.enabled)
    return {
      tone: "idle",
      text: pending > 0 ? `Paused · ${pending} waiting` : "Paused",
    };
  if (pending > 0 && connection.lastError) {
    const wait = service.queue.nextAttemptAt
      ? Math.max(0, service.queue.nextAttemptAt - now)
      : 0;
    return {
      tone: "warn",
      text: `${pending} waiting · retrying ${formatWait(wait)}`,
    };
  }
  if (pending > 0) return { tone: "ok", text: `${pending} sending` };
  if (connection.lastScrobbledAt) {
    return {
      tone: "ok",
      text: `Last scrobble ${formatScrobbleAgo(now - connection.lastScrobbledAt)}`,
    };
  }
  return { tone: "ok", text: "Connected" };
}

function formatWait(seconds: number): string {
  if (seconds < 60) return "in a moment";
  if (seconds < 3600) return `in ${Math.round(seconds / 60)} min`;
  return `in ${Math.round(seconds / 3600)} h`;
}

export function formatScrobbleAgo(seconds: number): string {
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
  if (seconds < 86_400) return `${Math.floor(seconds / 3600)} h ago`;
  return `${Math.floor(seconds / 86_400)} d ago`;
}
