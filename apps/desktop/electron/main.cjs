const {
  app,
  BrowserWindow,
  dialog,
  ipcMain,
  Menu,
  powerMonitor,
  safeStorage,
  session,
  shell,
} = require("electron");
const { execFile, spawn } = require("node:child_process");
const os = require("node:os");
const { randomBytes } = require("node:crypto");
const fs = require("node:fs");
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
  nearestExistingFolder,
  normalizeServerAddress,
  probeMessage,
  probeServer,
  saveConfig,
  serverRows,
  startupPlan,
  useLocalServer,
  useRemoteServer,
  isThisComputer,
} = require("./server-config.cjs");
const { createServerMonitor } = require("./server-monitor.cjs");
const {
  createServerService,
  servicePort,
  serviceKind,
} = require("./server-service.cjs");
const { runServerHost } = require("./server-host.cjs");
const {
  createSessionVault,
  electronCookieStore,
  fileVaultStorage,
} = require("./session-vault.cjs");

// `Parson --server-host` is the background process that keeps the server
// running (macOS LaunchAgent, Linux without systemd). It has no window.
const SERVER_HOST = process.argv.includes("--server-host");

// "Use this computer" runs the server as a background service; this window
// is then a remote for it on loopback, like any other device.
const LOCAL_PORT = servicePort(process.env);
const LOCAL_ORIGIN = `http://127.0.0.1:${LOCAL_PORT}`;
let backendInstanceKey = "";
const ALLOWED_LOCAL_PAGES = [
  "startup.html",
  "startup-error.html",
  "servers.html",
].map((name) => pathToFileURL(path.join(__dirname, name)).href);
const SERVERS_PAGE = path.join(__dirname, "servers.html");
const STARTUP_POLL_MS = 150;
const LOCAL_START_TIMEOUT_MS = 45_000;
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
let serviceManager = null;
let vault = null;
let upgradeChecked = false;

const primary = !SERVER_HOST && app.requestSingleInstanceLock();
if (!SERVER_HOST && !primary) {
  app.quit();
} else if (!SERVER_HOST) {
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

function trayHostExecutable() {
  return app.isPackaged
    ? path.join(process.resourcesPath, "ParsonMusicServer.exe")
    : path.join(__dirname, "bin", "ParsonMusicServer.exe");
}

function runCommand(command, args) {
  return new Promise((resolve) => {
    execFile(
      command,
      args,
      { timeout: 20_000, windowsHide: true, encoding: "utf8" },
      (error, stdout, stderr) => {
        resolve({
          code: error ? (typeof error.code === "number" ? error.code : -1) : 0,
          stdout: stdout || "",
          stderr: stderr || (error && !stdout ? String(error.message) : ""),
        });
      },
    );
  });
}

/** The manager for the server's background service on this computer. */
async function localService() {
  if (serviceManager) return serviceManager;
  const platform = process.platform;
  const systemdUser =
    platform === "linux" &&
    (await runCommand("systemctl", ["--user", "show-environment"])).code === 0;
  const kind = serviceKind({
    platform,
    systemdUser,
    trayAvailable: fs.existsSync(trayHostExecutable()),
  });
  serviceManager = createServerService({
    platform,
    env: process.env,
    home: os.homedir(),
    uid: typeof process.getuid === "function" ? process.getuid() : 0,
    kind,
    execPath: process.execPath,
    appPath: app.getAppPath(),
    packaged: app.isPackaged,
    appVersion: app.getVersion(),
    // A development run as root (containers) needs the same sandbox switch.
    hostArgs:
      !app.isPackaged && process.argv.includes("--no-sandbox")
        ? ["--no-sandbox"]
        : [],
    bundledServer: backendExecutable(),
    bundledTray: trayHostExecutable(),
    logsDir: platform === "darwin" ? app.getPath("logs") : undefined,
    run: runCommand,
    spawnDetached: (command, args, env) => {
      const child = spawn(command, args, {
        detached: true,
        stdio: "ignore",
        windowsHide: true,
        env: { ...process.env, ...env },
      });
      child.on("error", (error) =>
        console.error("Could not start the Parson server", error),
      );
      child.unref();
    },
  });
  return serviceManager;
}

function localServerAvailable() {
  if (!fs.existsSync(backendExecutable())) return false;
  return process.platform !== "win32" || fs.existsSync(trayHostExecutable());
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

async function probeBackend(origin = LOCAL_ORIGIN) {
  const challenge = randomBytes(32).toString("hex");
  try {
    const response = await fetch(`${origin}/.well-known/parson`, {
      signal: AbortSignal.timeout(1_500),
      cache: "no-store",
      headers: { "x-parson-desktop-challenge": challenge },
    });
    if (!response.ok) {
      return { kind: "other" };
    }
    const manifest = await response.json();
    if (isTrustedBackendManifest(manifest, backendInstanceKey, challenge)) {
      return { kind: "trusted", manifest };
    }
    return isParsonManifest(manifest)
      ? { kind: "untrusted-parson", manifest }
      : { kind: "other" };
  } catch {
    return { kind: "unreachable" };
  }
}

/**
 * Makes sure the server runs as this computer's background service and
 * returns its manifest. The service, not this window, owns the server:
 * quitting Parson Desktop leaves it running for every other device.
 */
async function ensureLocalServer() {
  const service = await localService();
  if (!service.available) {
    throw new Error(
      "This copy of Parson cannot run the server in the background on this computer.",
    );
  }
  const current = await probeBackend();
  const outdated =
    current.kind === "trusted" &&
    current.manifest.serverVersion !== app.getVersion() &&
    !upgradeChecked;
  if (current.kind === "trusted" && !outdated) {
    // Already running. Refresh the service definition quietly (an app that
    // moved, a new key) without interrupting playback.
    void service
      .ensure({ token: backendInstanceKey })
      .catch((error) => console.error("Could not refresh the service", error));
    return current.manifest;
  }
  if (current.kind === "other") {
    throw new Error(
      `Port ${LOCAL_PORT} is used by another program, so the Parson server cannot start. Close that program or connect to a Parson server on your network instead.`,
    );
  }
  upgradeChecked = true;
  await service.ensure({ token: backendInstanceKey, upgrade: outdated });
  const deadline = Date.now() + LOCAL_START_TIMEOUT_MS;
  let foreignSince = 0;
  while (Date.now() < deadline) {
    const status = await probeBackend();
    if (status.kind === "trusted") return status.manifest;
    if (status.kind === "untrusted-parson") {
      // The service reads its key file on each request, so a server we
      // installed proves itself at once. One that keeps failing is a
      // different Parson server (another data folder) on this port.
      foreignSince ||= Date.now();
      if (Date.now() - foreignSince > 5_000) {
        const error = new Error(
          `Another Parson server (“${status.manifest.name}”) already runs on port ${LOCAL_PORT} of this computer. Connect to it from “Connect to a server on my network”, or stop it first.`,
        );
        error.foreign = status.manifest;
        throw error;
      }
    } else {
      foreignSince = 0;
    }
    await new Promise((resolve) => setTimeout(resolve, STARTUP_POLL_MS));
  }
  const where =
    service.kind === "systemd-user"
      ? " Check `systemctl --user status parson-server` and `journalctl --user -u parson-server`."
      : "";
  throw new Error(
    `The Parson server did not start within ${LOCAL_START_TIMEOUT_MS / 1000} seconds.${where}`,
  );
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
  let manifest;
  try {
    manifest = await ensureLocalServer();
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    if (error?.foreign) {
      chooserWarning = {
        title: "Another Parson server already runs on this computer. ",
        body: detail,
      };
      serverMode = null;
      await showChooser();
      return false;
    }
    await showStartupFailure(classifyStartupFailure(detail), detail);
    return false;
  }
  if (manifest.instanceId !== serverSettings.localInstanceId) {
    updateServerSettings({
      ...serverSettings,
      localInstanceId: manifest.instanceId,
    });
  }
  appOrigin = LOCAL_ORIGIN;
  currentRemote = {
    id: manifest.instanceId,
    instanceId: manifest.instanceId,
    name: "this computer",
    origin: LOCAL_ORIGIN,
    bootId: typeof manifest.bootId === "string" ? manifest.bootId : "",
    local: true,
  };
  await loadRemoteApp();
  watchRemote();
  return true;
}

async function loadRemoteApp() {
  chooserView = "choose";
  reconnectState = null;
  try {
    await vault?.activate(currentRemote.instanceId, currentRemote.origin);
  } catch (error) {
    console.error("Could not restore this server's sign-in", error);
  }
  try {
    await mainWindow.loadURL(resumeTarget(currentRemote.origin));
  } catch (error) {
    console.error("Could not load the Parson app", error);
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
        local: Boolean(server.local),
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
  if (isThisComputer(manifest.instanceId, serverSettings.localInstanceId)) {
    // This is the server on this computer, reached by its network address:
    // use it as "this computer" instead of becoming a remote of ourselves
    // (which would also misreport that no server runs here).
    if (serverMode === "local") {
      await loadRemoteApp();
      return { ok: true };
    }
    if (localServerAvailable()) {
      const ok = await startLocal({ persist });
      return ok
        ? { ok }
        : { ok, message: "This computer's server could not start." };
    }
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

function currentRows() {
  return serverRows({
    config: serverSettings,
    discovered,
    localInstanceId: serverSettings.localInstanceId,
    activeId:
      serverMode === "remote" && currentRemote
        ? currentRemote.instanceId || currentRemote.id
        : null,
    // The connected row is the server at this address, even when another
    // server answers with the same identity.
    activeOrigin:
      serverMode === "remote" && currentRemote ? currentRemote.origin : "",
    // The window is connected unless the monitor lost the server.
    activeOnline: chooserView !== "reconnecting" && !reconnectState,
  });
}

async function serversState() {
  const rows = currentRows();
  // Sign-in is per server identity, wherever the server was reached.
  const signedIn = vault ? await vault.signedIn() : new Set();
  const current =
    currentRemote && chooserView !== "reconnecting"
      ? { name: currentRemote.name, origin: currentRemote.origin }
      : null;
  return {
    view: chooserView,
    mode: serverMode,
    current,
    warning: chooserWarning,
    reconnect: reconnectState,
    local: {
      available: localServerAvailable(),
      // The background server keeps running while this window shows the
      // chooser or another server.
      running: (await probeBackend()).kind === "trusted",
      signedIn: Boolean(
        serverSettings.localInstanceId &&
        signedIn.has(serverSettings.localInstanceId),
      ),
    },
    rows: rows.map((row) => ({
      ...row,
      signedIn: Boolean(row.instanceId && signedIn.has(row.instanceId)),
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
      // Keyed by address: two servers can answer with the same identity
      // (a library taken over by a restore while the old server still
      // runs), and the chooser must see both, whichever answered first.
      reachable.set(normalized, {
        instanceId: result.manifest.instanceId,
        name: result.manifest.name,
        origin: normalized,
        serverVersion: result.manifest.serverVersion,
        identityAdoptedAt: result.manifest.identityAdoptedAt ?? 0,
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
      if (!localServerAvailable()) {
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
        // A forgotten server that is still on the network stays listed
        // (signed out), so it can be connected to again right away.
        updateServerSettings(forgetServer(serverSettings, server.id));
        // Sign out of that server, wherever it was reached.
        try {
          if (server.instanceId) await vault?.forget(server.instanceId);
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
      if (serverMode === "local") {
        await startLocal({ persist: false });
      } else if (serverMode === "remote" && currentRemote) {
        await connectRemote(currentRemote, { persist: false });
      }
      return true;
    case "retry":
      if (currentRemote?.local) {
        // The window never owns the server; ask the service to run it.
        try {
          await (await localService()).start();
        } catch (error) {
          console.error("Could not start the background server", error);
        }
      }
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
          localAvailable: localServerAvailable(),
          // Whether this computer's own background server is running, so a
          // remote-mode window never claims that no server runs here.
          localRunning:
            serverMode === "local" ||
            (localServerAvailable() &&
              (await probeBackend()).kind === "trusted"),
        };
      case "server_service":
        return (await localService()).status();
      case "server_service_set": {
        const service = await localService();
        if (typeof args.autostart === "boolean") {
          await service.setAutostart(args.autostart);
        }
        if (args.action === "restart") await service.restart();
        if (args.action === "stop") {
          await service.stop();
          // This window was a remote for it; show the server's absence
          // honestly instead of a frozen library.
          void monitor?.check();
        }
        if (args.action === "start") {
          await service.start();
          void monitor?.check();
        }
        return service.status();
      }
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
        // "Locate…" for a moved folder opens near where the music was.
        const near =
          typeof args.near === "string" ? nearestExistingFolder(args.near) : "";
        const selection = await dialog.showOpenDialog(mainWindow, {
          properties: ["openDirectory", "createDirectory"],
          title: near ? "Where is this music now?" : "Choose your music folder",
          ...(near ? { defaultPath: near } : {}),
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

if (SERVER_HOST) {
  runServerHost({
    electron: require("electron"),
    serverExecutable: backendExecutable(),
    iconPath: app.isPackaged
      ? path.join(process.resourcesPath, "parson.png")
      : path.join(__dirname, "../../web/public/icons/icon-512.png"),
    service: {
      status: async () => (await localService()).status(),
      setAutostart: async (enabled) =>
        (await localService()).setAutostart(enabled),
    },
  });
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
      vault = createSessionVault({
        store: electronCookieStore(session.defaultSession.cookies),
        ...fileVaultStorage({
          file: path.join(app.getPath("userData"), "server-sessions.bin"),
          safeStorage,
        }),
      });
      // Keep each server's sign-in current as the web app refreshes it.
      let captureTimer = null;
      session.defaultSession.cookies.on("changed", () => {
        if (!currentRemote) return;
        clearTimeout(captureTimer);
        const origin = currentRemote.origin;
        captureTimer = setTimeout(() => void vault.capture(origin), 400);
      });
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
        // The server runs as its own background service, outside the files
        // an update replaces, so installing an update never stops it. After
        // the update, the new app restarts an older server once.
        stopBackend: () => Promise.resolve(),
        restartBackend: () => Promise.resolve(),
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
  if (!SERVER_HOST && BrowserWindow.getAllWindows().length === 0) {
    void createWindow().catch((error) => {
      const detail = error instanceof Error ? error.message : String(error);
      console.error("Parson window restoration failed", error);
      dialog.showErrorBox("Parson could not restore its window", detail);
    });
  }
});
app.on("window-all-closed", () => {
  if (!SERVER_HOST && process.platform !== "darwin") {
    app.quit();
  }
});
// Quitting the window never stops the server: it is a background service
// that phones, browsers and other computers keep using. Only the current
// server's sign-in is saved on the way out.
let savedOnQuit = false;
app.on("before-quit", (event) => {
  if (SERVER_HOST) return;
  quitting = true;
  stopMonitor();
  if (savedOnQuit || !vault || !currentRemote) return;
  event.preventDefault();
  savedOnQuit = true;
  const timeout = new Promise((resolve) => setTimeout(resolve, 1_500));
  void Promise.race([vault.capture(currentRemote.origin), timeout]).finally(
    () => app.quit(),
  );
});
