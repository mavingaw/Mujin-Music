const {
  app,
  BrowserWindow,
  dialog,
  ipcMain,
  Menu,
  powerMonitor,
  session,
  shell,
} = require("electron");
const { spawn } = require("node:child_process");
const { randomBytes } = require("node:crypto");
const fs = require("node:fs");
const net = require("node:net");
const path = require("node:path");
const { pathToFileURL } = require("node:url");
const {
  APPLICATION_ID,
  compareVersions,
  getInstallState,
  installAppImage,
  integrateInstallation,
} = require("./linux-installer.cjs");
const { relaunchInstalledApp } = require("./linux-relaunch.cjs");
const { isAllowedNavigation } = require("./navigation.cjs");
const {
  isParsonManifest,
  isTrustedBackendManifest,
} = require("./backend-trust.cjs");
const { classifyStartupFailure } = require("./failure-state.cjs");
const { createUpdates, updateSupport } = require("./updates.cjs");
const { browse } = require("./mdns.cjs");
const {
  commandAllowed,
  forgetServer,
  loadConfig,
  normalizeServerAddress,
  probeMessage,
  probeServer,
  saveConfig,
  serverRows,
  startupPlan,
  useLocalServer,
  useRemoteServer,
} = require("./server-config.cjs");
const { createServerMonitor } = require("./server-monitor.cjs");

const DEFAULT_PORT = 1993;
let backendPort = DEFAULT_PORT;
let backendOrigin = `http://127.0.0.1:${DEFAULT_PORT}`;
let backendInstanceKey = "";
const ALLOWED_LOCAL_PAGES = [
  "startup.html",
  "startup-error.html",
  "servers.html",
].map((name) => pathToFileURL(path.join(__dirname, name)).href);
const SERVERS_PAGE = path.join(__dirname, "servers.html");
const STARTUP_POLL_MS = 10;
let backend = null;
let mainWindow = null;
let quitting = false;
let updates = null;

// Server/remote split. `serverMode` is "local" when this app runs the Parson
// server sidecar, "remote" when it is a remote for a server elsewhere (and
// never starts one), and null while the user is choosing. `appOrigin` is the
// origin of the Parson web app the window shows.
let serverMode = null;
let appOrigin = "";
let currentRemote = null;
let serverSettings = null;
let serverSettingsExist = false;
let hadLocalInstall = false;
let discovered = [];
let discovering = null;
let chooserView = "choose";
let chooserWarning = null;
let reconnectState = null;
let monitor = null;
let resumeUrl = null;

const primary = app.requestSingleInstanceLock();
if (!primary) {
  app.quit();
} else {
  app.on("second-instance", () => showMainWindow());
}

function showMainWindow() {
  if (!mainWindow || mainWindow.isDestroyed()) {
    return;
  }
  if (mainWindow.isMinimized()) {
    mainWindow.restore();
  }
  mainWindow.show();
  mainWindow.focus();
}

function backendExecutable() {
  const executable = `parson-music-server${process.platform === "win32" ? ".exe" : ""}`;
  if (app.isPackaged) {
    return path.join(process.resourcesPath, executable);
  }
  return path.join(__dirname, "bin", executable);
}

function refreshDesktopIntegration(applicationsDirectory) {
  for (const [command, args] of [
    ["update-desktop-database", [applicationsDirectory]],
    [
      "xdg-mime",
      ["default", `${APPLICATION_ID}.desktop`, "x-scheme-handler/parson"],
    ],
  ]) {
    const child = spawn(command, args, {
      detached: true,
      stdio: "ignore",
    });
    child.on("error", () => {});
    child.unref();
  }
}

async function offerLinuxInstallation() {
  if (process.platform !== "linux" || !app.isPackaged) {
    return false;
  }
  const state = getInstallState();
  if (!state.available) {
    return false;
  }

  const iconSource = path.join(process.resourcesPath, "parson.png");
  const version = app.getVersion();
  if (state.isCanonical) {
    try {
      await integrateInstallation({ iconSource, version });
    } catch (error) {
      console.error("Could not refresh Linux desktop integration", error);
    }
    return false;
  }

  const comparison = state.installedVersion
    ? compareVersions(state.installedVersion, version)
    : null;
  const alreadyCurrent = state.installed && comparison === 0;
  const installedIsNewer = state.installed && comparison === 1;
  const action =
    alreadyCurrent || installedIsNewer
      ? "Open installed"
      : state.installed
        ? "Update"
        : "Install";
  const versionDetail = installedIsNewer
    ? `A newer version (${state.installedVersion}) is already installed.`
    : alreadyCurrent
      ? `Version ${version} is already installed.`
      : state.installedVersion
        ? `Version ${state.installedVersion} is installed.`
        : state.installed
          ? "An existing installation was found."
          : "Install Parson for your user account.";
  const choice = process.argv.includes("--install")
    ? 0
    : dialog.showMessageBoxSync({
        type: "info",
        title: `${action} Parson`,
        message:
          alreadyCurrent || installedIsNewer
            ? versionDetail
            : `${action} Parson ${version}?`,
        detail:
          alreadyCurrent || installedIsNewer
            ? "You can open the installed copy or run this file once."
            : `${versionDetail}\n\nNo administrator password is required. Parson will add itself to your applications and restart.`,
        buttons: [action, "Run once"],
        defaultId: 0,
        cancelId: 1,
      });
  if (choice !== 0) {
    return false;
  }

  try {
    const paths =
      alreadyCurrent || installedIsNewer
        ? await integrateInstallation({
            iconSource,
            version: state.installedVersion,
          })
        : await installAppImage({
            iconSource,
            sourcePath: state.source,
            version,
          });
    refreshDesktopIntegration(path.dirname(paths.desktopEntry));
    relaunchInstalledApp({
      application: paths.application,
      electronApp: app,
    });
    return true;
  } catch (error) {
    dialog.showMessageBoxSync({
      type: "error",
      title: `${action} failed`,
      message:
        alreadyCurrent || installedIsNewer
          ? "Parson could not open the installed copy."
          : `Parson could not ${action.toLowerCase()} itself.`,
      detail: error instanceof Error ? error.message : String(error),
      buttons: ["Continue"],
    });
    return false;
  }
}

function rotateBackendLog(logPath) {
  try {
    if (fs.statSync(logPath).size <= 2 * 1024 * 1024) {
      return;
    }
    fs.renameSync(logPath, `${logPath}.previous`);
  } catch (error) {
    if (error?.code !== "ENOENT") {
      console.error("Could not rotate backend log", error);
    }
  }
}

function backendFailureDetail(logPath, status, startOffset = 0) {
  try {
    const contents = fs.readFileSync(logPath, "utf8");
    const currentRun = contents.slice(startOffset);
    const tail = currentRun.slice(-8_000).trim();
    return tail ? `${status}\n\nRecent backend log:\n${tail}` : status;
  } catch {
    return status;
  }
}

function loadDesktopInstanceKey() {
  const keyPath = path.join(app.getPath("userData"), "desktop-instance.key");
  let invalidKeyPath = null;
  try {
    const existing = fs.readFileSync(keyPath, "utf8").trim();
    if (/^[a-f0-9]{64}$/i.test(existing)) {
      return existing.toLowerCase();
    }
    invalidKeyPath = `${keyPath}.invalid-${Date.now()}`;
    fs.renameSync(keyPath, invalidKeyPath);
  } catch (error) {
    if (error?.code !== "ENOENT") {
      throw new Error(`Could not read the desktop identity key: ${error}`);
    }
  }
  fs.mkdirSync(path.dirname(keyPath), { recursive: true });
  const key = randomBytes(32).toString("hex");
  const temporary = `${keyPath}.${process.pid}.tmp`;
  try {
    fs.writeFileSync(temporary, `${key}\n`, {
      encoding: "utf8",
      flag: "wx",
      mode: 0o600,
    });
    fs.renameSync(temporary, keyPath);
  } catch (error) {
    try {
      fs.rmSync(temporary);
    } catch (cleanupError) {
      if (cleanupError?.code !== "ENOENT") {
        console.error(
          "Could not remove the temporary desktop identity key",
          cleanupError,
        );
      }
    }
    if (invalidKeyPath) {
      try {
        fs.renameSync(invalidKeyPath, keyPath);
      } catch (restoreError) {
        console.error(
          "Could not restore the invalid desktop identity key",
          restoreError,
        );
      }
    }
    throw new Error(`Could not create the desktop identity key: ${error}`);
  }
  try {
    fs.chmodSync(keyPath, 0o600);
  } catch (error) {
    if (process.platform !== "win32") {
      throw error;
    }
  }
  return key;
}

async function probeBackend(origin = backendOrigin) {
  const challenge = randomBytes(32).toString("hex");
  try {
    const response = await fetch(`${origin}/.well-known/parson`, {
      signal: AbortSignal.timeout(750),
      cache: "no-store",
      headers: { "x-parson-desktop-challenge": challenge },
    });
    if (!response.ok) {
      return { kind: "other" };
    }
    const manifest = await response.json();
    if (isTrustedBackendManifest(manifest, backendInstanceKey, challenge)) {
      return { kind: "trusted" };
    }
    return { kind: isParsonManifest(manifest) ? "untrusted-parson" : "other" };
  } catch {
    return { kind: "unreachable" };
  }
}

function portIsListening(port) {
  return new Promise((resolve) => {
    const socket = net.createConnection({ host: "127.0.0.1", port });
    const finish = (listening) => {
      socket.removeAllListeners();
      socket.destroy();
      resolve(listening);
    };
    socket.setTimeout(500);
    socket.once("connect", () => finish(true));
    socket.once("timeout", () => finish(false));
    socket.once("error", () => finish(false));
  });
}

function availableLoopbackPort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = typeof address === "object" && address ? address.port : 0;
      server.close((error) => {
        if (error) {
          reject(error);
        } else if (!port) {
          reject(new Error("Could not allocate a local port."));
        } else {
          resolve(port);
        }
      });
    });
  });
}

async function startBackend() {
  if (backend && !backend.killed) {
    const current = await probeBackend(backendOrigin);
    if (current.kind === "trusted") {
      return;
    }
    throw new Error(
      "The running Parson backend did not pass its desktop authentication check.",
    );
  }
  const defaultOrigin = `http://127.0.0.1:${DEFAULT_PORT}`;
  const existing = await probeBackend(defaultOrigin);
  if (existing.kind === "trusted") {
    backendPort = DEFAULT_PORT;
    backendOrigin = defaultOrigin;
    return;
  }
  if (existing.kind === "untrusted-parson") {
    throw new Error(
      "Port 1993 is occupied by a Parson server that this desktop app did not start. Stop that server before opening Parson Desktop.",
    );
  }
  if (
    existing.kind === "unreachable" &&
    (await portIsListening(DEFAULT_PORT))
  ) {
    throw new Error(
      "Port 1993 is occupied by a service that did not answer Parson's desktop authentication check. Stop that service or configure it to use another port.",
    );
  }
  if (existing.kind === "other") {
    backendPort = await availableLoopbackPort();
    backendOrigin = `http://127.0.0.1:${backendPort}`;
  } else {
    backendPort = DEFAULT_PORT;
    backendOrigin = defaultOrigin;
  }
  const executable = backendExecutable();
  if (!fs.existsSync(executable)) {
    throw new Error(`The packaged Parson backend is missing: ${executable}`);
  }
  const logsDirectory = app.getPath("logs");
  fs.mkdirSync(logsDirectory, { recursive: true });
  const logPath = path.join(logsDirectory, "backend.log");
  rotateBackendLog(logPath);
  const logStartOffset = fs.existsSync(logPath) ? fs.statSync(logPath).size : 0;
  const log = fs.openSync(logPath, "a");
  try {
    backend = spawn(executable, [], {
      cwd: path.dirname(executable),
      windowsHide: true,
      env: {
        ...process.env,
        PARSON_BIND_ADDRESS: "0.0.0.0",
        PARSON_DESKTOP_INSTANCE_TOKEN: backendInstanceKey,
        // This computer's audio devices (USB DACs, outputs) become zones.
        PARSON_LOCAL_AUDIO: process.env.PARSON_LOCAL_AUDIO || "on",
        PARSON_PORT: String(backendPort),
        PARSON_PUBLIC_URL: backendOrigin,
      },
      stdio: ["ignore", log, log],
    });
  } finally {
    fs.closeSync(log);
  }
  let backendStartError = null;
  backend.once("error", (error) => {
    backendStartError = error;
    backend = null;
  });
  backend.once("exit", (code, signal) => {
    backend = null;
    if (!quitting && serverMode === "local" && !mainWindow?.isDestroyed()) {
      const detail = backendFailureDetail(
        logPath,
        `Backend stopped (${code ?? signal ?? "unknown"}).`,
        logStartOffset,
      );
      void showStartupFailure(classifyStartupFailure(detail), detail);
    }
  });
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    const status = await probeBackend();
    if (status.kind === "trusted") {
      return;
    }
    if (status.kind === "untrusted-parson") {
      throw new Error(
        "The local Parson backend failed desktop authentication.",
      );
    }
    if (backendStartError) {
      throw new Error(
        `The Parson backend could not start: ${backendStartError.message}`,
      );
    }
    if (!backend) {
      throw new Error("The Parson backend stopped during startup.");
    }
    await new Promise((resolve) => setTimeout(resolve, STARTUP_POLL_MS));
  }
  throw new Error("The Parson backend did not become ready within 30 seconds.");
}

async function showStartupFailure(kind, detail) {
  if (!mainWindow || mainWindow.isDestroyed()) {
    return;
  }
  await mainWindow.loadFile(path.join(__dirname, "startup-error.html"), {
    query: { detail: String(detail || ""), kind },
  });
}

async function loadApplication() {
  const plan = startupPlan({
    config: serverSettings,
    configExists: serverSettingsExist,
    hasLocalLibrary: hadLocalInstall,
    env: process.env,
    argv: process.argv,
  });
  if (plan.kind === "local") {
    return startLocal({ persist: false });
  }
  if (plan.kind === "remote") {
    await mainWindow.loadFile(path.join(__dirname, "startup.html"));
    const result = await connectRemote(plan.server, {
      persist: !plan.forced,
      reconnectIfDown: true,
    });
    return result.ok;
  }
  if (!serverSettingsExist) {
    // Remember that this install has been offered the choice, so quitting
    // at the chooser does not later look like an existing local install.
    updateServerSettings(serverSettings);
  }
  await showChooser();
  return false;
}

function updateServerSettings(next) {
  serverSettings = next;
  serverSettingsExist = true;
  try {
    saveConfig(app.getPath("userData"), next);
  } catch (error) {
    console.error("Could not save the server choice", error);
  }
}

function currentAppUrl() {
  if (!mainWindow || mainWindow.isDestroyed()) return null;
  const url = mainWindow.webContents.getURL();
  try {
    return appOrigin && new URL(url).origin === appOrigin ? url : null;
  } catch {
    return null;
  }
}

function sendToWindow(channel, payload) {
  if (mainWindow && !mainWindow.isDestroyed()) {
    mainWindow.webContents.send(channel, payload);
  }
}

function stopMonitor() {
  monitor?.stop();
  monitor = null;
}

function resumeTarget(origin) {
  const target = resumeUrl?.startsWith(`${origin}/`) ? resumeUrl : origin;
  resumeUrl = null;
  return target;
}

async function startLocal({ persist = true } = {}) {
  stopMonitor();
  if (serverMode !== "local") resumeUrl = null;
  serverMode = "local";
  appOrigin = "";
  currentRemote = null;
  chooserWarning = null;
  chooserView = "choose";
  reconnectState = null;
  if (persist) updateServerSettings(useLocalServer(serverSettings));
  await mainWindow.loadFile(path.join(__dirname, "startup.html"));
  try {
    await startBackend();
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    await showStartupFailure(classifyStartupFailure(detail), detail);
    return false;
  }
  appOrigin = backendOrigin;
  const identity = await probeServer(backendOrigin);
  if (
    identity.kind === "ok" &&
    identity.manifest.instanceId !== serverSettings.localInstanceId
  ) {
    updateServerSettings({
      ...serverSettings,
      localInstanceId: identity.manifest.instanceId,
    });
  }
  try {
    await mainWindow.loadURL(resumeTarget(appOrigin));
    return true;
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    await showStartupFailure("webview_failed", detail);
    return false;
  }
}

async function loadRemoteApp() {
  chooserView = "choose";
  reconnectState = null;
  try {
    await mainWindow.loadURL(resumeTarget(currentRemote.origin));
  } catch (error) {
    console.error("Could not load the remote Parson app", error);
  }
}

function watchRemote({ down = false } = {}) {
  stopMonitor();
  const server = currentRemote;
  if (!server) return;
  monitor = createServerMonitor({
    probe: () => probeServer(server.origin),
    instanceId: server.instanceId,
    bootId: server.bootId,
    initiallyDown: down,
    onDown: ({ lastSeenAt }) => {
      resumeUrl = currentAppUrl() ?? resumeUrl;
      reconnectState = {
        name: server.name,
        origin: server.origin,
        lastSeenAt,
        attempt: 0,
        nextAt: Date.now() + 1_000,
        local: false,
      };
      chooserView = "reconnecting";
      void mainWindow.loadFile(SERVERS_PAGE);
    },
    onAttempt: ({ attempt, nextInMs }) => {
      if (!reconnectState) return;
      reconnectState = {
        ...reconnectState,
        attempt,
        nextAt: Date.now() + nextInMs,
      };
      void publishServersState();
    },
    onUp: () => {
      void loadRemoteApp();
    },
    onRestart: (manifest) => {
      // The server restarted between two probes. The web app's live
      // connections reconnect on their own; tell it to refetch everything.
      server.bootId = manifest.bootId;
      sendToWindow("parson:server-event", { type: "restarted" });
    },
    onReplaced: (manifest) => {
      chooserWarning = {
        title: "This address now belongs to a different Parson server. ",
        body: `${server.origin} answers as “${manifest.name || "another library"}”, not “${server.name}”. Choose it below if you trust it.`,
      };
      void showChooser();
    },
  });
  monitor.start();
}

/**
 * Connects this window to a Parson server on the network. Never starts the
 * local server; stops it if it was running.
 */
async function connectRemote(
  target,
  { persist = true, trustNewIdentity = false, reconnectIfDown = false } = {},
) {
  const origin = normalizeServerAddress(target?.origin);
  if (!origin) {
    return { ok: false, message: "Enter a valid server address." };
  }
  const result = await probeServer(origin);
  if (result.kind !== "ok") {
    if (reconnectIfDown && result.kind === "unreachable") {
      // Startup with a saved server that is offline: wait for it rather
      // than dropping the user into the chooser.
      if (backend) await stopBackendForRetry();
      serverMode = "remote";
      appOrigin = origin;
      currentRemote = {
        id: target.id || target.instanceId || origin,
        instanceId: target.instanceId || "",
        name: target.name || new URL(origin).host,
        origin,
        bootId: "",
      };
      reconnectState = {
        name: currentRemote.name,
        origin,
        lastSeenAt: target.lastConnectedAt || 0,
        attempt: 0,
        nextAt: Date.now() + 1_000,
        local: false,
      };
      chooserView = "reconnecting";
      await mainWindow.loadFile(SERVERS_PAGE);
      watchRemote({ down: true });
    }
    return { ok: false, message: probeMessage(result, origin) };
  }
  const manifest = result.manifest;
  if (
    serverSettings.localInstanceId &&
    manifest.instanceId === serverSettings.localInstanceId &&
    serverMode === "local"
  ) {
    // This is the server this app already runs, reached by its network
    // address: stay on it instead of becoming a remote of ourselves.
    await mainWindow.loadURL(resumeTarget(appOrigin));
    return { ok: true };
  }
  if (
    !trustNewIdentity &&
    target.instanceId &&
    manifest.instanceId !== target.instanceId
  ) {
    chooserWarning = {
      title: "This address now belongs to a different Parson server. ",
      body: `${new URL(origin).host} now answers as “${manifest.name}”. Connect to it from the list if you trust it.`,
    };
    await showChooser();
    return { ok: false, message: chooserWarning.title.trim() };
  }
  stopMonitor();
  if (backend) await stopBackendForRetry();
  if (serverMode !== "remote" || appOrigin !== origin) resumeUrl = null;
  serverMode = "remote";
  appOrigin = origin;
  chooserWarning = null;
  currentRemote = {
    id: manifest.instanceId,
    instanceId: manifest.instanceId,
    name: manifest.name || new URL(origin).host,
    origin,
    bootId: typeof manifest.bootId === "string" ? manifest.bootId : "",
  };
  if (persist) {
    updateServerSettings(
      useRemoteServer(serverSettings, {
        instanceId: manifest.instanceId,
        name: manifest.name,
        origin,
        serverVersion: manifest.serverVersion,
        manual: Boolean(target.manual),
      }),
    );
  }
  await loadRemoteApp();
  watchRemote();
  return { ok: true };
}

async function showChooser() {
  stopMonitor();
  if (chooserView !== "reconnecting") {
    resumeUrl = currentAppUrl() ?? resumeUrl;
  }
  chooserView = "choose";
  reconnectState = null;
  await mainWindow.loadFile(SERVERS_PAGE);
}

async function signedInOrigins(origins) {
  const signedIn = new Set();
  await Promise.all(
    origins.map(async (origin) => {
      try {
        const cookies = await session.defaultSession.cookies.get({
          url: origin,
          name: "plm_refreshToken",
        });
        if (cookies.length) signedIn.add(origin);
      } catch {}
    }),
  );
  return signedIn;
}

function currentRows() {
  return serverRows({
    config: serverSettings,
    discovered,
    localInstanceId: serverSettings.localInstanceId,
  });
}

async function serversState() {
  const rows = currentRows();
  const signedIn = await signedInOrigins(rows.map((row) => row.origin));
  const current =
    serverMode === "remote" && currentRemote && chooserView !== "reconnecting"
      ? { name: currentRemote.name, origin: currentRemote.origin }
      : serverMode === "local" && backend
        ? { name: "this computer", origin: backendOrigin }
        : null;
  return {
    view: chooserView,
    mode: serverMode,
    current,
    warning: chooserWarning,
    reconnect: reconnectState,
    local: { available: fs.existsSync(backendExecutable()) },
    rows: rows.map((row) => ({
      ...row,
      signedIn: signedIn.has(row.origin),
      active: serverMode === "remote" && currentRemote?.origin === row.origin,
    })),
  };
}

async function publishServersState() {
  if (!mainWindow || mainWindow.isDestroyed()) return;
  try {
    if (new URL(mainWindow.webContents.getURL()).protocol !== "file:") return;
  } catch {
    return;
  }
  mainWindow.webContents.send("parson:servers-state", await serversState());
}

/**
 * Looks for Parson servers: mDNS answers plus every saved server, each
 * confirmed by asking it for its manifest. Results stream to the chooser.
 */
function discoverServers() {
  if (discovering) return discovering;
  discovering = (async () => {
    const reachable = new Map();
    const checked = new Set();
    const check = async (origin) => {
      const normalized = normalizeServerAddress(origin);
      if (!normalized || checked.has(normalized)) return;
      checked.add(normalized);
      const result = await probeServer(normalized, { timeoutMs: 1_800 });
      if (result.kind !== "ok") return;
      reachable.set(result.manifest.instanceId, {
        instanceId: result.manifest.instanceId,
        name: result.manifest.name,
        origin: normalized,
        serverVersion: result.manifest.serverVersion,
      });
      discovered = [...reachable.values()];
      void publishServersState();
    };
    const pending = serverSettings.servers.map((server) =>
      check(server.origin),
    );
    await browse({
      durationMs: 2_400,
      onService: (service) => pending.push(check(service.origin)),
    });
    await Promise.allSettled(pending);
    discovered = [...reachable.values()];
  })().finally(() => {
    discovering = null;
  });
  return discovering;
}

async function handleServersAction(action, args = {}) {
  switch (action) {
    case "state":
      return serversState();
    case "discover":
      await discoverServers();
      return serversState();
    case "connect": {
      if (typeof args.address === "string") {
        const result = await connectRemote({
          origin: args.address,
          manual: true,
        });
        return result.ok ? result : { ...result, state: await serversState() };
      }
      const row = currentRows().find((candidate) => candidate.id === args.id);
      if (!row) {
        return { ok: false, message: "That server is no longer listed." };
      }
      const result = await connectRemote(row, {
        // A server just confirmed on the network, or one the user picked
        // from the list after a warning, is trusted as it answers now.
        trustNewIdentity: row.nearby || Boolean(chooserWarning),
      });
      return result.ok ? result : { ...result, state: await serversState() };
    }
    case "use-local": {
      if (!fs.existsSync(backendExecutable())) {
        return {
          ok: false,
          message: "This copy of Parson does not include the server.",
        };
      }
      const ok = await startLocal();
      return ok ? { ok } : { ok, message: "The server could not start." };
    }
    case "forget": {
      const server = serverSettings.servers.find(
        (candidate) => candidate.id === args.id,
      );
      if (server) {
        updateServerSettings(forgetServer(serverSettings, server.id));
        discovered = discovered.filter(
          (entry) =>
            entry.instanceId !== server.instanceId &&
            entry.origin !== server.origin,
        );
        // Per-server sign-in lives in that origin's cookies and storage.
        try {
          await session.defaultSession.clearStorageData({
            origin: server.origin,
          });
        } catch (error) {
          console.error("Could not clear the forgotten server's data", error);
        }
      }
      return serversState();
    }
    case "back":
      if (serverMode === "remote" && currentRemote) {
        await connectRemote(currentRemote, { persist: false });
      } else if (serverMode === "local" && backend) {
        appOrigin = backendOrigin;
        await mainWindow.loadURL(resumeTarget(appOrigin));
      }
      return true;
    case "retry":
      if (monitor) await monitor.check();
      return true;
    case "choose":
      stopMonitor();
      chooserView = "choose";
      reconnectState = null;
      return serversState();
    default:
      throw new Error(`Unknown server action: ${action}`);
  }
}

async function stopBackendForRetry() {
  const processToStop = backend;
  if (!processToStop || processToStop.killed) {
    return;
  }
  backend = null;
  processToStop.removeAllListeners("exit");
  processToStop.removeAllListeners("error");
  processToStop.once("error", () => {});
  await new Promise((resolve) => {
    const timeout = setTimeout(resolve, 1_500);
    processToStop.once("exit", () => {
      clearTimeout(timeout);
      resolve();
    });
    processToStop.kill("SIGTERM");
  });
}

function installDesktopBridge() {
  ipcMain.handle("parson:window-control", (event, action) => {
    if (
      !isAllowedNavigation(
        event.senderFrame.url,
        appOrigin,
        ALLOWED_LOCAL_PAGES,
      )
    ) {
      throw new Error("Window controls are unavailable to untrusted content.");
    }
    const window = BrowserWindow.fromWebContents(event.sender);
    if (!window || window.isDestroyed()) {
      return false;
    }
    switch (action) {
      case "minimize":
        window.minimize();
        return true;
      case "toggle-maximize":
        if (window.isMaximized()) {
          window.unmaximize();
        } else {
          window.maximize();
        }
        return window.isMaximized();
      case "is-maximized":
        return window.isMaximized();
      case "close":
        window.close();
        return true;
      default:
        throw new Error(`Unknown window control: ${action}`);
    }
  });

  ipcMain.on("parson:server-mode", (event) => {
    try {
      event.returnValue =
        appOrigin && new URL(event.senderFrame.url).origin === appOrigin
          ? serverMode
          : null;
    } catch {
      event.returnValue = null;
    }
  });

  ipcMain.handle("parson:servers", async (event, action, args = {}) => {
    if (!isAllowedNavigation(event.senderFrame.url, "", ALLOWED_LOCAL_PAGES)) {
      throw new Error("Server selection is only available to Parson Desktop.");
    }
    return handleServersAction(action, args ?? {});
  });

  ipcMain.handle("parson:invoke", async (event, command, args = {}) => {
    if (!appOrigin || new URL(event.senderFrame.url).origin !== appOrigin) {
      throw new Error(
        "Desktop commands are only available to the Parson app this window shows.",
      );
    }
    if (!commandAllowed(command, serverMode)) {
      throw new Error(
        serverMode === "remote"
          ? `${command} is only available when Parson runs the server on this computer.`
          : `Unknown desktop command: ${command}`,
      );
    }
    switch (command) {
      case "server_state":
        return {
          mode: serverMode,
          name:
            serverMode === "remote"
              ? (currentRemote?.name ?? "")
              : "This computer",
          origin: appOrigin,
          localAvailable: fs.existsSync(backendExecutable()),
        };
      case "server_choose":
        setImmediate(() => void showChooser());
        return true;
      case "server_connect": {
        const origin = normalizeServerAddress(args.origin);
        if (!origin) throw new Error("A valid server address is required.");
        const result = await connectRemote({ origin, manual: true });
        if (!result.ok) throw new Error(result.message);
        return true;
      }
      case "server_use_local":
        setImmediate(() => void startLocal());
        return true;
      case "updates_state":
        return updates.state();
      case "updates_check":
        return updates.check();
      case "updates_install":
        return updates.install();
      case "platform":
        if (process.platform === "win32") {
          return "windows";
        }
        if (process.platform === "darwin") {
          return "macos";
        }
        return "linux";
      case "select_music_folder": {
        const selection = await dialog.showOpenDialog(mainWindow, {
          properties: ["openDirectory", "createDirectory"],
          title: "Choose your music folder",
        });
        return selection.canceled ? null : (selection.filePaths[0] ?? null);
      }
      case "show_track_in_file_manager": {
        const target = typeof args.path === "string" ? args.path : "";
        if (!target || !fs.existsSync(target)) {
          throw new Error("A valid track path is required.");
        }
        shell.showItemInFolder(target);
        return true;
      }
      default:
        throw new Error(`Unknown desktop command: ${command}`);
    }
  });

  ipcMain.handle("parson:startup-action", async (event, action) => {
    if (
      !isAllowedNavigation(
        event.senderFrame.url,
        appOrigin,
        ALLOWED_LOCAL_PAGES,
      )
    ) {
      throw new Error("Startup recovery is unavailable to untrusted content.");
    }
    if (action === "open-logs") {
      shell.openPath(app.getPath("logs"));
      return true;
    }
    if (action === "retry") {
      await stopBackendForRetry();
      return loadApplication();
    }
    throw new Error(`Unknown startup action: ${action}`);
  });
}

async function createWindow() {
  mainWindow = new BrowserWindow({
    width: 1280,
    height: 820,
    minWidth: 900,
    minHeight: 600,
    show: true,
    backgroundColor: "#000000",
    autoHideMenuBar: true,
    frame: process.platform === "darwin",
    title: "Parson",
    icon: app.isPackaged
      ? path.join(process.resourcesPath, "parson.png")
      : path.join(__dirname, "../../web/public/icons/icon-512.png"),
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      preload: path.join(__dirname, "preload.cjs"),
    },
  });
  const publishMaximizedState = () => {
    if (!mainWindow?.isDestroyed()) {
      mainWindow.webContents.send("parson:maximized", mainWindow.isMaximized());
    }
  };
  mainWindow.on("maximize", publishMaximizedState);
  mainWindow.on("unmaximize", publishMaximizedState);
  mainWindow.maximize();
  mainWindow.webContents.on("before-input-event", (event, input) => {
    if (input.type !== "keyDown" || input.isAutoRepeat) {
      return;
    }
    if (input.key === "F11") {
      event.preventDefault();
      mainWindow.setFullScreen(!mainWindow.isFullScreen());
    } else if (input.key === "Escape" && mainWindow.isFullScreen()) {
      event.preventDefault();
      mainWindow.setFullScreen(false);
    }
  });
  mainWindow.webContents.setWindowOpenHandler(({ url }) => {
    try {
      const candidate = new URL(url);
      if (candidate.protocol === "http:" || candidate.protocol === "https:") {
        void shell.openExternal(candidate.href);
      }
    } catch {}
    return { action: "deny" };
  });
  mainWindow.webContents.on("will-navigate", (event, url) => {
    if (isAllowedNavigation(url, appOrigin, ALLOWED_LOCAL_PAGES)) {
      return;
    }
    event.preventDefault();
  });
  mainWindow.webContents.on("render-process-gone", (_event, details) => {
    if (quitting || mainWindow?.isDestroyed()) {
      return;
    }
    void showStartupFailure(
      "webview_failed",
      `Renderer stopped: ${details.reason}`,
    );
  });
  await loadApplication();
}

if (primary) {
  app
    .whenReady()
    .then(async () => {
      app.setName("Parson");
      hadLocalInstall = fs.existsSync(
        path.join(app.getPath("userData"), "desktop-instance.key"),
      );
      ({ config: serverSettings, exists: serverSettingsExist } = loadConfig(
        app.getPath("userData"),
      ));
      backendInstanceKey = loadDesktopInstanceKey();
      if (process.platform !== "darwin") {
        Menu.setApplicationMenu(null);
      }
      if (await offerLinuxInstallation()) {
        return;
      }
      app.setAsDefaultProtocolClient("parson");
      installDesktopBridge();
      const metadata = require(path.join(app.getAppPath(), "package.json"));
      const unsupported = updateSupport({
        packaged: app.isPackaged,
        platform: process.platform,
        environment: process.env,
        macSigned:
          metadata.parsonUpdates?.macSigned === true ||
          metadata.parsonUpdates?.macSigned === "true",
      });
      updates = createUpdates({
        updater: unsupported ? null : require("electron-updater").autoUpdater,
        version: app.getVersion(),
        unsupported,
        publish: (state) => {
          if (mainWindow && !mainWindow.isDestroyed()) {
            mainWindow.webContents.send("parson:updates", state);
          }
        },
        stopBackend: stopBackendForRetry,
        // A remote never runs the server, not even after an update attempt.
        restartBackend: () =>
          serverMode === "local" ? startBackend() : Promise.resolve(),
      });
      const publishPowerResume = () => {
        void monitor?.check();
        if (!mainWindow?.isDestroyed()) {
          mainWindow.webContents.send("parson:power-resume");
        }
      };
      powerMonitor.on("resume", publishPowerResume);
      powerMonitor.on("unlock-screen", publishPowerResume);
      await createWindow();
      if (!unsupported) {
        const initialCheck = setTimeout(() => void updates.check(), 15000);
        initialCheck.unref();
        const periodicCheck = setInterval(
          () => void updates.check(),
          6 * 60 * 60 * 1000,
        );
        periodicCheck.unref();
      }
    })
    .catch((error) => {
      const detail = error instanceof Error ? error.message : String(error);
      console.error("Parson desktop initialization failed", error);
      dialog.showErrorBox("Parson could not start", detail);
      app.quit();
    });
}

app.on("activate", () => {
  if (BrowserWindow.getAllWindows().length === 0) {
    void createWindow().catch((error) => {
      const detail = error instanceof Error ? error.message : String(error);
      console.error("Parson window restoration failed", error);
      dialog.showErrorBox("Parson could not restore its window", detail);
    });
  }
});
app.on("window-all-closed", () => {
  if (process.platform !== "darwin") {
    app.quit();
  }
});
app.on("before-quit", (event) => {
  quitting = true;
  if (backend && !backend.killed) {
    event.preventDefault();
    void stopBackendForRetry().finally(() => app.quit());
  }
});
