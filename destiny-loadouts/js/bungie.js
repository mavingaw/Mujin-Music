// Bungie.net sign-in (OAuth) and API requests.
//
// Everything is stored in this browser's localStorage only: your API key,
// OAuth client details and tokens never leave your machine except to talk to
// bungie.net.

const ROOT = "https://www.bungie.net";
const AUTHORIZE_URL = `${ROOT}/en/OAuth/Authorize`;
const TOKEN_URL = `${ROOT}/platform/app/oauth/token/`;

const CONFIG_KEY = "d2lo.config";
const TOKENS_KEY = "d2lo.tokens";
const STATE_KEY = "d2lo.oauthState";

export class BungieError extends Error {
  constructor(message, status, data) {
    super(message);
    this.status = status;
    this.data = data;
  }
}

function readJson(key) {
  try {
    return JSON.parse(localStorage.getItem(key) ?? "null");
  } catch {
    return null;
  }
}

export function getConfig() {
  return readJson(CONFIG_KEY);
}

export function saveConfig(config) {
  localStorage.setItem(CONFIG_KEY, JSON.stringify(config));
}

export function clearConfig() {
  localStorage.removeItem(CONFIG_KEY);
}

/** The URL to register as the OAuth redirect URL: this page, no query. */
// Set by the desktop app (desktop/preload.cjs); absent in a normal browser.
const desktop = globalThis.desktop;
export const isDesktop = Boolean(desktop?.isDesktop);

/** The URL to register as the OAuth redirect URL. */
export function redirectUrl() {
  return isDesktop
    ? desktop.redirectUrl
    : `${location.origin}${location.pathname}`;
}

/** The value to register as the Origin Header. */
export function appOrigin() {
  return isDesktop ? desktop.origin : location.origin;
}

/**
 * fetch(), or in the desktop app a request made by the app itself (which adds
 * the Origin header and avoids browser CORS rules). Returns { ok, status, json }.
 */
async function http(url, { method = "GET", headers = {}, body } = {}) {
  if (!isDesktop) {
    const response = await fetch(url, { method, headers, body });
    return {
      ok: response.ok,
      status: response.status,
      json: () => response.json(),
    };
  }
  const result = await desktop.request({
    url,
    method,
    headers,
    body: body === undefined ? undefined : String(body),
  });
  return {
    ok: result.status >= 200 && result.status < 300,
    status: result.status,
    json: async () => JSON.parse(result.body),
  };
}

function getTokens() {
  return readJson(TOKENS_KEY);
}

function saveTokens(response) {
  const now = Date.now();
  const tokens = {
    access: response.access_token,
    accessExpires: now + response.expires_in * 1000,
    refresh: response.refresh_token ?? null,
    refreshExpires: response.refresh_token
      ? now + response.refresh_expires_in * 1000
      : 0,
    membershipId: response.membership_id,
  };
  localStorage.setItem(TOKENS_KEY, JSON.stringify(tokens));
  return tokens;
}

export function signOut() {
  localStorage.removeItem(TOKENS_KEY);
}

export function isSignedIn() {
  const tokens = getTokens();
  if (!tokens) return false;
  const now = Date.now();
  return tokens.accessExpires > now || tokens.refreshExpires > now;
}

/** Sends the browser to Bungie's sign-in page. */
/**
 * Starts Bungie sign-in. In a browser this leaves the page and Bungie sends
 * it back with ?code=...; in the desktop app a sign-in window opens and this
 * resolves once you are signed in.
 */
export async function beginSignIn() {
  const config = getConfig();
  const state = crypto.randomUUID();
  const params = new URLSearchParams({
    client_id: config.clientId,
    response_type: "code",
    state,
  });
  const url = `${AUTHORIZE_URL}?${params}`;
  if (!isDesktop) {
    sessionStorage.setItem(STATE_KEY, state);
    location.assign(url);
    return;
  }
  const returned = new URLSearchParams(await desktop.signIn(url));
  if (returned.get("state") !== state || !returned.get("code")) {
    throw new BungieError(
      returned.get("error_description") || "Sign-in didn't finish. Try again.",
    );
  }
  await requestToken({
    grant_type: "authorization_code",
    code: returned.get("code"),
  });
}

async function requestToken(fields) {
  const config = getConfig();
  const body = new URLSearchParams({ ...fields, client_id: config.clientId });
  if (config.clientSecret) body.set("client_secret", config.clientSecret);
  const response = await http(TOKEN_URL, {
    method: "POST",
    body,
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
  });
  const data = await response.json().catch(() => ({}));
  if (!response.ok || !data.access_token) {
    throw new BungieError(
      data.error_description ||
        data.error ||
        `Sign-in failed (${response.status})`,
      response.status,
      data,
    );
  }
  return saveTokens(data);
}

/**
 * Finishes sign-in when Bungie sends the browser back with ?code=...
 * Returns true when it handled a redirect.
 */
export async function completeSignInFromUrl() {
  const params = new URLSearchParams(location.search);
  const code = params.get("code");
  if (!code) return false;
  const expected = sessionStorage.getItem(STATE_KEY);
  sessionStorage.removeItem(STATE_KEY);
  history.replaceState(null, "", redirectUrl());
  if (!expected || params.get("state") !== expected) {
    throw new BungieError(
      "The sign-in response did not match this browser's request. Try again.",
    );
  }
  await requestToken({ grant_type: "authorization_code", code });
  return true;
}

let refreshing = null;

async function accessToken({ forceRefresh = false } = {}) {
  const tokens = getTokens();
  if (!tokens) throw new BungieError("Not signed in", 401);
  const now = Date.now();
  if (!forceRefresh && tokens.accessExpires - 60_000 > now) {
    return tokens.access;
  }
  if (!tokens.refresh || tokens.refreshExpires < now) {
    signOut();
    throw new BungieError("Your sign-in expired. Sign in again.", 401);
  }
  refreshing ??= requestToken({
    grant_type: "refresh_token",
    refresh_token: tokens.refresh,
  }).finally(() => {
    refreshing = null;
  });
  return (await refreshing).access;
}

const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * Calls a Bungie.net Platform endpoint and returns its Response field.
 * path starts after /Platform, e.g. "/User/GetMembershipsForCurrentUser/".
 */
export async function api(path, { method = "GET", body, auth = true } = {}) {
  const config = getConfig();
  let refreshedOnce = false;
  for (let attempt = 0; attempt < 4; attempt++) {
    const headers = { "X-API-Key": config.apiKey };
    if (auth) headers.Authorization = `Bearer ${await accessToken()}`;
    if (body) headers["Content-Type"] = "application/json";

    const response = await http(`${ROOT}/Platform${path}`, {
      method,
      headers,
      body: body ? JSON.stringify(body) : undefined,
    });
    const data = await response.json().catch(() => null);

    if (response.status === 401 && auth && !refreshedOnce) {
      refreshedOnce = true;
      await accessToken({ forceRefresh: true });
      continue;
    }
    if (data && data.ErrorCode === 1) return data.Response;

    const throttled =
      data?.ThrottleSeconds > 0 || /throttl/i.test(data?.ErrorStatus ?? "");
    if (throttled && attempt < 3) {
      await wait(Math.max(1, data?.ThrottleSeconds ?? 1) * 1000);
      continue;
    }
    throw new BungieError(
      data?.Message || `Bungie.net request failed (${response.status})`,
      response.status,
      data,
    );
  }
  throw new BungieError("Bungie.net kept throttling the request. Try again.");
}

/** Fetches a file from bungie.net (manifest tables), not the Platform API. */
export async function fetchContent(path) {
  const response = await http(`${ROOT}${path}`);
  if (!response.ok) {
    throw new BungieError(
      `Download failed (${response.status})`,
      response.status,
    );
  }
  return response.json();
}

export function imageUrl(path) {
  return path ? `${ROOT}${path}` : "";
}
