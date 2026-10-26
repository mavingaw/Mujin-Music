// Server/remote split for Parson Desktop.
//
// The desktop app is either the whole system ("local": it runs the Parson
// server sidecar on this computer, which other devices can also use) or only
// a remote ("remote": it connects to a Parson server elsewhere on the network
// and never starts a server of its own). This module holds the pure logic for
// that choice so it can be tested without Electron.
const fs = require("node:fs");
const path = require("node:path");

const CONFIG_VERSION = 1;
const DEFAULT_PORT = 1993;
const MAX_SERVERS = 20;
const MODES = new Set(["local", "remote"]);

function normalizeServerAddress(value) {
  const trimmed = String(value ?? "").trim();
  if (!trimmed || trimmed.length > 512) return "";
  let candidate = /^[a-z][a-z\d+.-]*:\/\//i.test(trimmed)
    ? trimmed
    : `http://${trimmed}`;
  // A bare IPv6 address ("fd00::42") needs brackets to be a URL host.
  const bare = candidate.slice(candidate.indexOf("//") + 2).split("/")[0];
  if (/^[\da-f:]+$/i.test(bare) && bare.split(":").length > 2) {
    candidate = candidate.replace(bare, `[${bare}]`);
  }
  try {
    const url = new URL(candidate);
    if (url.protocol !== "http:" && url.protocol !== "https:") return "";
    if (url.username || url.password || !url.hostname) return "";
    if (url.protocol === "http:" && !url.port) url.port = String(DEFAULT_PORT);
    return url.origin;
  } catch {
    return "";
  }
}

function isLoopbackOrigin(origin) {
  try {
    const host = new URL(origin).hostname;
    return (
      host === "localhost" ||
      host === "[::1]" ||
      /^127\.\d+\.\d+\.\d+$/.test(host)
    );
  } catch {
    return false;
  }
}

function text(value, max = 256) {
  if (typeof value !== "string") return "";
  const trimmed = value.trim();
  return trimmed.length <= max ? trimmed : trimmed.slice(0, max);
}

function sanitizeServer(value) {
  if (!value || typeof value !== "object") return null;
  const origin = normalizeServerAddress(value.origin);
  if (!origin) return null;
  const instanceId = text(value.instanceId, 128);
  return {
    id: instanceId || origin,
    instanceId,
    name: text(value.name) || new URL(origin).hostname,
    origin,
    serverVersion: text(value.serverVersion, 64),
    lastConnectedAt: Number.isFinite(value.lastConnectedAt)
      ? value.lastConnectedAt
      : 0,
    manual: value.manual === true,
  };
}

function emptyConfig() {
  return {
    version: CONFIG_VERSION,
    mode: null,
    activeServerId: null,
    localInstanceId: "",
    servers: [],
  };
}

function parseConfig(value) {
  if (!value || typeof value !== "object") return emptyConfig();
  const servers = [];
  const seen = new Set();
  for (const entry of Array.isArray(value.servers) ? value.servers : []) {
    const server = sanitizeServer(entry);
    if (!server || seen.has(server.id)) continue;
    seen.add(server.id);
    servers.push(server);
    if (servers.length >= MAX_SERVERS) break;
  }
  const mode = MODES.has(value.mode) ? value.mode : null;
  const activeServerId = servers.some(
    (server) => server.id === value.activeServerId,
  )
    ? value.activeServerId
    : null;
  return {
    version: CONFIG_VERSION,
    mode,
    activeServerId,
    localInstanceId: text(value.localInstanceId, 128),
    servers,
  };
}

function configPath(userData) {
  return path.join(userData, "servers.json");
}

function loadConfig(userData) {
  try {
    return {
      exists: true,
      config: parseConfig(
        JSON.parse(fs.readFileSync(configPath(userData), "utf8")),
      ),
    };
  } catch (error) {
    return { exists: error?.code !== "ENOENT", config: emptyConfig() };
  }
}

function saveConfig(userData, config) {
  const target = configPath(userData);
  fs.mkdirSync(userData, { recursive: true });
  const temporary = `${target}.${process.pid}.tmp`;
  fs.writeFileSync(
    temporary,
    `${JSON.stringify(parseConfig(config), null, 2)}\n`,
    {
      encoding: "utf8",
      mode: 0o600,
    },
  );
  fs.renameSync(temporary, target);
}

function activeServer(config) {
  return (
    config.servers.find((server) => server.id === config.activeServerId) ?? null
  );
}

function argumentValue(argv, name) {
  const prefix = `--${name}=`;
  const inline = argv.find((argument) => argument.startsWith(prefix));
  if (inline) return inline.slice(prefix.length);
  const index = argv.indexOf(`--${name}`);
  return index >= 0 ? (argv[index + 1] ?? "") : "";
}

/**
 * What to do when the app starts.
 * - `--server=<address>` / PARSON_DESKTOP_SERVER: connect to that server.
 * - `--local` / PARSON_DESKTOP_MODE=local: run the server here.
 * - `--choose-server`: always show the chooser.
 * - a saved choice wins next;
 * - with no saved choice, an existing install (which has always run its own
 *   server) keeps doing so, and a first run asks.
 */
function startupPlan({
  config,
  configExists = false,
  hasLocalLibrary = false,
  env = {},
  argv = [],
}) {
  const forcedAddress =
    argumentValue(argv, "server") || env.PARSON_DESKTOP_SERVER || "";
  if (forcedAddress) {
    const origin = normalizeServerAddress(forcedAddress);
    if (origin) {
      const saved = config.servers.find((server) => server.origin === origin);
      return {
        kind: "remote",
        server: saved ?? sanitizeServer({ origin, manual: true }),
        forced: true,
      };
    }
    return { kind: "choose", reason: "invalid-address" };
  }
  if (argv.includes("--choose-server")) {
    return { kind: "choose", reason: "requested" };
  }
  if (argv.includes("--local") || env.PARSON_DESKTOP_MODE === "local") {
    return { kind: "local", forced: true };
  }
  if (config.mode === "local") return { kind: "local" };
  if (config.mode === "remote") {
    const server = activeServer(config);
    return server
      ? { kind: "remote", server }
      : { kind: "choose", reason: "no-server" };
  }
  if (!configExists && hasLocalLibrary) return { kind: "local" };
  return { kind: "choose", reason: "first-run" };
}

/** Saves a server after a successful connection, following it if it moved. */
function rememberServer(config, details, now = Date.now()) {
  const incoming = sanitizeServer({ ...details, lastConnectedAt: now });
  if (!incoming) return config;
  const existing = config.servers.find(
    (server) =>
      (incoming.instanceId && server.instanceId === incoming.instanceId) ||
      server.origin === incoming.origin,
  );
  const merged = existing
    ? {
        ...existing,
        ...incoming,
        id: incoming.instanceId || existing.id,
        manual: existing.manual || incoming.manual,
      }
    : incoming;
  const servers = [
    merged,
    ...config.servers.filter(
      (server) =>
        server !== existing &&
        server.id !== merged.id &&
        server.origin !== merged.origin,
    ),
  ].slice(0, MAX_SERVERS);
  return { ...config, servers };
}

function useRemoteServer(config, details, now = Date.now()) {
  const next = rememberServer(config, details, now);
  const server = next.servers[0];
  return { ...next, mode: "remote", activeServerId: server?.id ?? null };
}

function useLocalServer(config) {
  return { ...config, mode: "local", activeServerId: null };
}

function forgetServer(config, id) {
  const servers = config.servers.filter((server) => server.id !== id);
  const forgettingActive = config.activeServerId === id;
  return {
    ...config,
    servers,
    activeServerId: forgettingActive ? null : config.activeServerId,
    mode: forgettingActive && config.mode === "remote" ? null : config.mode,
  };
}

/**
 * The rows the chooser shows: saved servers merged with what discovery just
 * found. A saved server that turns up at a new address (DHCP) is matched by
 * its instance id and shown at the new address.
 */
function serverRows({
  config,
  discovered = [],
  localInstanceId = "",
  signedIn = new Set(),
}) {
  const rows = new Map();
  for (const server of config.servers) {
    rows.set(server.id, {
      ...server,
      saved: true,
      nearby: false,
      active: config.mode === "remote" && server.id === config.activeServerId,
    });
  }
  for (const found of discovered) {
    const origin = normalizeServerAddress(found.origin);
    if (!origin) continue;
    const instanceId = text(found.instanceId, 128);
    if (instanceId && instanceId === localInstanceId) continue;
    const match =
      (instanceId && rows.get(instanceId)) ||
      [...rows.values()].find((row) => row.origin === origin);
    if (match) {
      rows.set(match.id, {
        ...match,
        origin,
        name: text(found.name) || match.name,
        serverVersion: text(found.serverVersion, 64) || match.serverVersion,
        nearby: true,
      });
      continue;
    }
    const id = instanceId || origin;
    rows.set(id, {
      id,
      instanceId,
      origin,
      name: text(found.name) || new URL(origin).hostname,
      serverVersion: text(found.serverVersion, 64),
      lastConnectedAt: 0,
      manual: false,
      saved: false,
      nearby: true,
      active: false,
    });
  }
  return [...rows.values()]
    .map((row) => ({ ...row, signedIn: signedIn.has(row.origin) }))
    .sort(
      (left, right) =>
        Number(right.active) - Number(left.active) ||
        Number(right.nearby) - Number(left.nearby) ||
        Number(right.saved) - Number(left.saved) ||
        right.lastConnectedAt - left.lastConnectedAt ||
        left.name.localeCompare(right.name),
    );
}

/** Reconnect schedule: quick at first, then every 15 seconds. */
function reconnectDelay(attempt) {
  const steps = [1_000, 2_000, 3_000, 5_000, 8_000];
  return steps[Math.max(0, attempt)] ?? 15_000;
}

// Commands the web app may send to the desktop shell. A remote server's web
// app is still Parson, but it runs on another machine, so commands that act
// on this computer's files are only offered to the server this app started.
const COMMANDS = Object.freeze({
  platform: "any",
  updates_state: "any",
  updates_check: "any",
  updates_install: "any",
  server_state: "any",
  server_choose: "any",
  server_connect: "any",
  server_use_local: "any",
  // The background server on this computer (start/stop/launch at startup).
  server_service: "any",
  server_service_set: "any",
  select_music_folder: "local",
  show_track_in_file_manager: "local",
});

function commandAllowed(command, mode) {
  const scope = COMMANDS[command];
  if (!scope) return false;
  return scope === "any" || mode === "local";
}

function isCompatibleManifest(manifest) {
  return (
    manifest?.protocol === "parson" &&
    manifest?.product === "parson-music" &&
    manifest?.protocolVersion === 1 &&
    typeof manifest?.instanceId === "string" &&
    manifest.instanceId.length > 0
  );
}

/**
 * Asks `origin` whether it is a Parson server.
 * Returns `{ kind: "ok", manifest }`, `{ kind: "incompatible", manifest }`,
 * `{ kind: "not-parson" }` or `{ kind: "unreachable", message }`.
 */
async function probeServer(
  origin,
  { fetch = globalThis.fetch, timeoutMs = 2_500 } = {},
) {
  try {
    const response = await fetch(`${origin}/.well-known/parson`, {
      cache: "no-store",
      headers: { Accept: "application/json" },
      signal: AbortSignal.timeout(timeoutMs),
    });
    if (!response.ok) return { kind: "not-parson" };
    const body = await response.text();
    if (body.length > 64 * 1024) return { kind: "not-parson" };
    let manifest;
    try {
      manifest = JSON.parse(body);
    } catch {
      return { kind: "not-parson" };
    }
    if (
      manifest?.protocol !== "parson" ||
      manifest?.product !== "parson-music"
    ) {
      return { kind: "not-parson" };
    }
    return isCompatibleManifest(manifest)
      ? { kind: "ok", manifest }
      : { kind: "incompatible", manifest };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return { kind: "unreachable", message };
  }
}

function probeMessage(result, origin) {
  const host = (() => {
    try {
      return new URL(origin).host;
    } catch {
      return origin;
    }
  })();
  switch (result.kind) {
    case "ok":
      return "";
    case "incompatible":
      return `${host} runs a version of Parson this app can’t talk to. Update both to the same version.`;
    case "not-parson":
      return `${host} answered, but it isn’t a Parson server. Check the address and port (Parson uses ${DEFAULT_PORT}).`;
    default:
      return `No Parson server answered at ${host}. Make sure it is running and on the same network, and that a firewall allows port ${new URL(origin).port || DEFAULT_PORT}.`;
  }
}

module.exports = {
  COMMANDS,
  DEFAULT_PORT,
  MAX_SERVERS,
  activeServer,
  commandAllowed,
  configPath,
  emptyConfig,
  forgetServer,
  isCompatibleManifest,
  isLoopbackOrigin,
  loadConfig,
  normalizeServerAddress,
  parseConfig,
  probeMessage,
  probeServer,
  reconnectDelay,
  rememberServer,
  saveConfig,
  serverRows,
  startupPlan,
  useLocalServer,
  useRemoteServer,
};
