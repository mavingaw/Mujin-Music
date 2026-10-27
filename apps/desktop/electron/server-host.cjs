// `Parson --server-host`: the background process that keeps the Parson
// server running on macOS (started by a LaunchAgent) and on Linux desktops
// without a systemd user session (started by XDG autostart).
//
// It has no window. It runs the bundled parson-music-server, restarts it if
// it crashes, and shows a menu-bar / tray status item with Running, Open
// Parson, Connect a phone, Launch at startup and Quit. The desktop window is
// only a remote for it, so closing that window never stops the server.
const { spawn } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const { servicePort } = require("./server-service.cjs");

const PORT = servicePort(process.env);
const CRASH_WINDOW_MS = 2 * 60_000;
const MAX_CRASHES = 5;

/** Wait before restarting after the nth crash in a row. */
function restartDelay(crashes) {
  return (
    [1_000, 2_000, 5_000, 10_000, 30_000][Math.max(0, crashes - 1)] ?? 30_000
  );
}

/** Whether to give up: too many crashes within the window. */
function shouldGiveUp(crashTimes, now = Date.now()) {
  return (
    crashTimes.filter((time) => now - time < CRASH_WINDOW_MS).length >=
    MAX_CRASHES
  );
}

function statusLabel(state) {
  switch (state.status) {
    case "running":
      return `Mujin Music Server — Running on port ${state.port ?? PORT}`;
    case "starting":
      return "Mujin Music Server — Starting…";
    case "restarting":
      return "Mujin Music Server — Restarting after a problem…";
    case "failed":
      return "Mujin Music Server — Stopped after repeated errors";
    case "stopping":
      return "Mujin Music Server — Stopping…";
    default:
      return "Mujin Music Server — Stopped";
  }
}

/**
 * The status item's menu as plain data; `actions` names the click handler
 * for each item so the model is testable without Electron.
 */
function hostMenuModel(state) {
  const running = state.status === "running";
  const stopped = state.status === "stopped" || state.status === "failed";
  return [
    { label: statusLabel(state), enabled: false },
    { type: "separator" },
    { label: "Open Mujin Music", action: "open-app" },
    {
      label: "Connect a phone or another computer…",
      action: "open-connections",
      enabled: running,
    },
    { type: "separator" },
    stopped
      ? { label: "Start server", action: "start" }
      : {
          label: "Restart server",
          action: "restart",
          enabled: running || state.status === "restarting",
        },
    {
      label: "Launch at startup",
      type: "checkbox",
      checked: Boolean(state.autostart),
      action: "toggle-autostart",
    },
    { label: "Open logs", action: "open-logs" },
    { type: "separator" },
    { label: "Quit Mujin Music Server", action: "quit" },
  ];
}

function tooltip(state) {
  return state.status === "running"
    ? "Mujin Music Server is running. Your phones and other computers can connect."
    : statusLabel(state);
}

/** Runs the host. Called from main.cjs before the normal app starts. */
function runServerHost({ electron, serverExecutable, service, iconPath }) {
  const { app, Menu, Tray, nativeImage, shell } = electron;
  // A separate profile, so this process and the desktop window each hold
  // their own single-instance lock.
  app.setPath("userData", path.join(app.getPath("appData"), "Parson Server"));
  if (!app.requestSingleInstanceLock()) {
    app.exit(0);
    return;
  }
  app.setName("Mujin Music Server");
  app.dock?.hide();

  const pidFile = path.join(app.getPath("userData"), "server-host.pid");
  const logsDir = app.getPath("logs");
  const state = { status: "starting", port: PORT, autostart: false };
  let child = null;
  let tray = null;
  let quitting = false;
  let restartTimer = null;
  const crashes = [];

  function refresh() {
    if (!tray) return;
    const template = hostMenuModel(state).map((item) =>
      item.type === "separator"
        ? item
        : {
            label: item.label,
            type: item.type === "checkbox" ? "checkbox" : "normal",
            checked: item.checked,
            enabled: item.enabled !== false,
            click: item.action ? () => void act(item.action) : undefined,
          },
    );
    tray.setContextMenu(Menu.buildFromTemplate(template));
    tray.setToolTip(tooltip(state));
  }

  function setStatus(status) {
    state.status = status;
    refresh();
  }

  function startServer() {
    clearTimeout(restartTimer);
    if (child) return;
    fs.mkdirSync(logsDir, { recursive: true });
    const log = fs.openSync(path.join(logsDir, "server.log"), "a");
    setStatus(state.status === "restarting" ? "restarting" : "starting");
    try {
      child = spawn(serverExecutable, [], {
        cwd: path.dirname(serverExecutable),
        env: {
          ...process.env,
          PARSON_PORT: String(PORT),
          PARSON_BIND_ADDRESS: process.env.PARSON_BIND_ADDRESS || "0.0.0.0",
        },
        stdio: ["ignore", log, log],
        windowsHide: true,
      });
    } finally {
      fs.closeSync(log);
    }
    const current = child;
    current.once("error", (error) => {
      console.error("Mujin Music server could not start", error);
    });
    current.once("exit", (code, signal) => {
      if (child === current) child = null;
      if (quitting || state.status === "stopping") {
        setStatus("stopped");
        return;
      }
      console.error(`Mujin Music server exited (${code ?? signal})`);
      crashes.push(Date.now());
      if (shouldGiveUp(crashes)) {
        setStatus("failed");
        return;
      }
      setStatus("restarting");
      restartTimer = setTimeout(
        startServer,
        restartDelay(
          crashes.filter((time) => Date.now() - time < CRASH_WINDOW_MS).length,
        ),
      );
    });
  }

  function stopServer() {
    clearTimeout(restartTimer);
    const current = child;
    if (!current) return Promise.resolve();
    setStatus("stopping");
    return new Promise((resolve) => {
      const force = setTimeout(() => current.kill("SIGKILL"), 10_000);
      current.once("exit", () => {
        clearTimeout(force);
        resolve();
      });
      current.kill("SIGTERM");
    });
  }

  async function probe() {
    try {
      const response = await fetch(
        `http://127.0.0.1:${PORT}/.well-known/parson`,
        {
          signal: AbortSignal.timeout(1_500),
        },
      );
      if (response.ok && child && state.status !== "stopping") {
        if (state.status !== "running") crashes.length = 0;
        setStatus("running");
      }
    } catch {}
  }

  async function act(action) {
    switch (action) {
      case "open-app":
        openApp();
        break;
      case "open-connections":
        void shell.openExternal(
          `http://127.0.0.1:${PORT}/settings?tab=connections`,
        );
        break;
      case "start":
        crashes.length = 0;
        startServer();
        break;
      case "restart":
        await stopServer();
        crashes.length = 0;
        startServer();
        break;
      case "toggle-autostart":
        try {
          await service?.setAutostart(!state.autostart);
          state.autostart = !state.autostart;
        } catch (error) {
          console.error("Could not change Launch at startup", error);
        }
        refresh();
        break;
      case "open-logs":
        void shell.openPath(logsDir);
        break;
      case "quit":
        void quit();
        break;
      default:
        break;
    }
  }

  function openApp() {
    if (process.platform === "darwin") {
      // Contents/MacOS/Parson -> Parson.app
      const bundle = path.resolve(process.execPath, "..", "..", "..");
      spawn("open", [bundle], { detached: true, stdio: "ignore" }).unref();
      return;
    }
    const program = process.env.APPIMAGE || process.execPath;
    const args = app.isPackaged ? [] : [app.getAppPath()];
    spawn(program, args, { detached: true, stdio: "ignore" }).unref();
  }

  async function quit() {
    if (quitting) return;
    quitting = true;
    await stopServer();
    try {
      fs.rmSync(pidFile, { force: true });
    } catch {}
    // Exit 0: launchd's KeepAlive (SuccessfulExit = false) leaves it stopped.
    app.exit(0);
  }

  process.on("SIGTERM", () => void quit());
  process.on("SIGINT", () => void quit());
  app.on("window-all-closed", () => {});
  app.on("before-quit", (event) => {
    if (!quitting) {
      event.preventDefault();
      void quit();
    }
  });

  app.whenReady().then(async () => {
    fs.mkdirSync(path.dirname(pidFile), { recursive: true });
    fs.writeFileSync(pidFile, `${process.pid}\n`);
    try {
      const image = nativeImage
        .createFromPath(iconPath)
        .resize({ width: 18, height: 18 });
      tray = new Tray(image);
    } catch (error) {
      // No status area (a bare X session): the server still runs.
      console.error("Mujin Music Server has no status item", error);
    }
    try {
      state.autostart = Boolean((await service?.status())?.autostart);
    } catch {}
    startServer();
    const timer = setInterval(probe, 3_000);
    timer.unref?.();
    void probe();
  });
}

module.exports = {
  CRASH_WINDOW_MS,
  MAX_CRASHES,
  hostMenuModel,
  restartDelay,
  runServerHost,
  shouldGiveUp,
  statusLabel,
};
