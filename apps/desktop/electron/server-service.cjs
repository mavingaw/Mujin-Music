// The Parson server as a background service, apart from the desktop window.
//
// When the user picks "Use this computer", Parson Desktop installs and starts
// the bundled server the way each platform runs background software, then
// connects to it on 127.0.0.1 like any other remote. Closing or quitting the
// desktop window never stops it, so phones, browsers and other computers keep
// their library and zones.
//
// - Windows: the Parson server tray host (crates/windows-server), copied to
//   %LOCALAPPDATA%\Programs\Parson Server and started at sign-in from the
//   HKCU Run key, the same entry its own "Start with Windows" item uses.
// - macOS: a LaunchAgent that runs Parson in --server-host mode, which shows
//   a menu-bar status item (Running, Launch at startup, Quit).
// - Linux: a systemd user unit derived from
//   crates/backend/packaging/parson-music.service; without a systemd user
//   session, an XDG autostart entry that runs --server-host instead.
//
// Everything that touches the system goes through `run`, `spawnDetached` and
// `fs`, so the logic is tested with fakes.
const nodeFs = require("node:fs");
const path = require("node:path");

const SERVICE_NAME = "parson-server";
const LAUNCH_AGENT_LABEL = "com.parsonlabs.parson.server";
const XDG_AUTOSTART_NAME = "com.parsonlabs.parson.server.desktop";
const RUN_KEY = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE = "ParsonMusicServer";
const TRAY_EXECUTABLE = "ParsonMusicServer.exe";
const HOST_FILE = "server.env";
const STAMP_FILE = "parson-desktop.json";
const DEFAULT_PORT = 1993;

const KIND_LABELS = Object.freeze({
  "windows-tray": "Parson Server (notification area)",
  "launch-agent": "Parson Server (menu bar)",
  "systemd-user": "systemd user service",
  "xdg-autostart": "Parson Server (background app)",
});

function joinFor(platform) {
  return platform === "win32" ? path.win32.join : path.posix.join;
}

/** Where the server keeps its database: the backend's default data dir. */
function serverDataDir({ platform, env = {}, home, join = joinFor(platform) }) {
  if (env.PARSON_DATA_DIR) return env.PARSON_DATA_DIR;
  if (platform === "win32") {
    return join(env.LOCALAPPDATA || join(home, "AppData", "Local"), "Parson");
  }
  if (platform === "darwin") {
    return join(home, "Library", "Application Support", "Parson");
  }
  return join(env.XDG_DATA_HOME || join(home, ".local", "share"), "Parson");
}

/** State of the --server-host process (pid file, Electron profile). */
function hostStateDir({ platform, env = {}, home, join = joinFor(platform) }) {
  if (platform === "darwin") {
    return join(home, "Library", "Application Support", "Parson Server");
  }
  if (platform === "win32") {
    return join(
      env.APPDATA || join(home, "AppData", "Roaming"),
      "Parson Server",
    );
  }
  return join(env.XDG_CONFIG_HOME || join(home, ".config"), "Parson Server");
}

/**
 * Merges the desktop's keys into `server.env`, keeping any line the user
 * wrote. `PARSON_LOCAL_AUDIO` is only added when absent, so someone who
 * turned device zones off keeps that choice.
 */
function hostFileContents(existing, { token, localAudio = "on" }) {
  const lines = String(existing || "")
    .split(/\r?\n/)
    .filter((line) => line.trim() !== "");
  const keyOf = (line) =>
    line.trim().startsWith("#") ? "" : line.split("=")[0].trim();
  const kept = lines.filter(
    (line) => keyOf(line) !== "PARSON_DESKTOP_INSTANCE_TOKEN",
  );
  const header = "# Written by Parson Desktop for the background server.";
  const body = kept.filter((line) => line !== header);
  if (
    !body.some((line) => keyOf(line) === "PARSON_LOCAL_AUDIO") &&
    localAudio
  ) {
    body.push(`PARSON_LOCAL_AUDIO=${localAudio}`);
  }
  body.push(`PARSON_DESKTOP_INSTANCE_TOKEN=${token}`);
  return `${[header, ...body].join("\n")}\n`;
}

/** The port the service listens on: 1993 unless PARSON_PORT says otherwise. */
function servicePort(env = {}) {
  const port = Number.parseInt(env.PARSON_PORT ?? "", 10);
  return Number.isInteger(port) && port > 0 && port < 65536
    ? port
    : DEFAULT_PORT;
}

// Settings that follow the desktop into the service when it was started
// with them (a second Parson on another port, a library on another disk).
function passthrough(env = {}) {
  return Object.fromEntries(
    ["PARSON_DATA_DIR", "PARSON_PORT"]
      .filter((key) => env[key])
      .map((key) => [key, String(env[key])]),
  );
}

function systemdQuote(value) {
  return `"${String(value).replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

/** A systemd user unit for the server (see parson-music.service). */
function systemdUnit({ executable, env = {} }) {
  const environment = [
    ["PARSON_PORT", String(servicePort(env))],
    // Listen on the network so phones and other computers can connect.
    ["PARSON_BIND_ADDRESS", "0.0.0.0"],
    ["PARSON_SERVICE_MANAGER", "systemd"],
    ...(env.PARSON_DATA_DIR ? [["PARSON_DATA_DIR", env.PARSON_DATA_DIR]] : []),
  ];
  return [
    "# Written by Parson Desktop when you chose “Use this computer”. It runs",
    "# the Parson server in the background, so phones, browsers and other",
    "# computers keep their library when the desktop window closes.",
    "# Based on crates/backend/packaging/parson-music.service.",
    "[Unit]",
    "Description=Parson music server",
    "After=network-online.target sound.target",
    "Wants=network-online.target",
    "",
    "[Service]",
    `ExecStart=${systemdQuote(executable)}`,
    ...environment.map(
      ([key, value]) => `Environment=${systemdQuote(`${key}=${value}`)}`,
    ),
    "Restart=on-failure",
    "RestartSec=3",
    "",
    "[Install]",
    "WantedBy=default.target",
    "",
  ].join("\n");
}

function xmlEscape(value) {
  return String(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

/** The LaunchAgent that keeps Parson's --server-host process running. */
function launchAgentPlist({ program, runAtLoad, logPath, env = {} }) {
  const variables = {
    PARSON_SERVICE_MANAGER: "launchd",
    ...passthrough(env),
  };
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${LAUNCH_AGENT_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
${program.map((argument) => `    <string>${xmlEscape(argument)}</string>`).join("\n")}
  </array>
  <key>EnvironmentVariables</key>
  <dict>
${Object.entries(variables)
  .map(
    ([key, value]) =>
      `    <key>${xmlEscape(key)}</key>\n    <string>${xmlEscape(value)}</string>`,
  )
  .join("\n")}
  </dict>
  <key>RunAtLoad</key>
  <${runAtLoad ? "true" : "false"}/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>ProcessType</key>
  <string>Interactive</string>
  <key>StandardOutPath</key>
  <string>${xmlEscape(logPath)}</string>
  <key>StandardErrorPath</key>
  <string>${xmlEscape(logPath)}</string>
</dict>
</plist>
`;
}

function desktopExecQuote(value) {
  return /[\s"\\`$]/.test(value)
    ? `"${String(value).replace(/(["\\`$])/g, "\\$1")}"`
    : value;
}

/** XDG autostart entry for desktops without a systemd user session. */
function xdgAutostartEntry({ program, enabled, env = {} }) {
  const variables = Object.entries(passthrough(env));
  const prefix = variables.length
    ? ["env", ...variables.map(([key, value]) => `${key}=${value}`)]
    : [];
  return [
    "[Desktop Entry]",
    "Type=Application",
    "Name=Parson Server",
    "Comment=Keeps your Parson library available to your phones and other computers",
    `Exec=${[...prefix, ...program].map(desktopExecQuote).join(" ")}`,
    "Icon=com.parsonlabs.parson",
    "Terminal=false",
    "NoDisplay=true",
    `X-GNOME-Autostart-enabled=${enabled ? "true" : "false"}`,
    `Hidden=${enabled ? "false" : "true"}`,
    "",
  ].join("\n");
}

function windowsRunCommand(executable) {
  return `"${executable}" --background`;
}

/**
 * Which background mechanism this computer gets. `systemdUser` is whether
 * `systemctl --user` answers; `trayAvailable` whether the Windows tray host
 * ships with this copy of Parson.
 */
function serviceKind({ platform, systemdUser = false, trayAvailable = false }) {
  if (platform === "win32") return trayAvailable ? "windows-tray" : null;
  if (platform === "darwin") return "launch-agent";
  if (platform === "linux")
    return systemdUser ? "systemd-user" : "xdg-autostart";
  return null;
}

/**
 * The command line that runs Parson in --server-host mode. An AppImage is
 * mounted at a new path on every launch, so the AppImage file itself is used.
 */
function serverHostProgram({
  execPath,
  appPath,
  packaged,
  env = {},
  extraArgs = [],
}) {
  if (packaged && env.APPIMAGE) {
    return [env.APPIMAGE, "--server-host", ...extraArgs];
  }
  if (packaged) return [execPath, "--server-host", ...extraArgs];
  return [execPath, appPath, "--server-host", ...extraArgs];
}

async function readText(fs, file) {
  try {
    return await fs.promises.readFile(file, "utf8");
  } catch (error) {
    if (error?.code === "ENOENT") return null;
    throw error;
  }
}

async function writeIfChanged(fs, file, contents, mode = 0o644) {
  if ((await readText(fs, file)) === contents) return false;
  await fs.promises.mkdir(path.dirname(file), { recursive: true });
  const temporary = `${file}.${process.pid}.tmp`;
  await fs.promises.writeFile(temporary, contents, { mode });
  await fs.promises.rename(temporary, file);
  return true;
}

/**
 * Creates the manager for this computer's background server.
 *
 * `bundledServer` is the parson-music-server binary shipped with the app and
 * `bundledTray` the Windows tray host (or null). `run(command, args)`
 * resolves to `{ code, stdout, stderr }`; `spawnDetached(command, args, env)`
 * starts a process that outlives this one.
 */
function createServerService({
  platform,
  env = {},
  home,
  uid = 0,
  kind,
  execPath,
  appPath,
  packaged = false,
  appVersion = "0.0.0",
  hostArgs = [],
  bundledServer,
  bundledTray = null,
  logsDir,
  run,
  spawnDetached,
  fs = nodeFs,
  sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
  isProcessAlive = (pid) => {
    try {
      process.kill(pid, 0);
      return true;
    } catch (error) {
      return error?.code === "EPERM";
    }
  },
  killProcess = (pid, signal) => process.kill(pid, signal),
}) {
  // Real files live on this machine, so its own path rules apply (tests run
  // every platform's logic on one host).
  const join = path.join;
  const dataDir = serverDataDir({ platform, env, home, join });
  const hostDir = hostStateDir({ platform, env, home, join });
  const program = serverHostProgram({
    execPath,
    appPath,
    packaged,
    env,
    extraArgs: hostArgs,
  });
  const configHome = env.XDG_CONFIG_HOME || join(home, ".config");
  const dataHome = env.XDG_DATA_HOME || join(home, ".local", "share");
  const paths = {
    hostFile: join(dataDir, HOST_FILE),
    unit: join(configHome, "systemd", "user", `${SERVICE_NAME}.service`),
    // An AppImage's files vanish when it exits, so a copy runs the service.
    linuxServer:
      packaged && env.APPIMAGE
        ? join(dataHome, "parson-server", "parson-music-server")
        : bundledServer,
    plist: join(home, "Library", "LaunchAgents", `${LAUNCH_AGENT_LABEL}.plist`),
    launchLog: join(
      logsDir || join(home, "Library", "Logs", "Parson"),
      "server-host.log",
    ),
    autostart: join(configHome, "autostart", XDG_AUTOSTART_NAME),
    hostPid: join(hostDir, "server-host.pid"),
    tray: join(
      env.LOCALAPPDATA || join(home, "AppData", "Local"),
      "Programs",
      "Parson Server",
      TRAY_EXECUTABLE,
    ),
  };
  const launchTarget = `gui/${uid}/${LAUNCH_AGENT_LABEL}`;

  const ok = async (command, args) => {
    const result = await run(command, args);
    if (result.code !== 0) {
      const detail = (result.stderr || result.stdout || "").trim();
      throw new Error(
        `${command} ${args.join(" ")} failed${detail ? `: ${detail}` : ` (${result.code})`}`,
      );
    }
    return result;
  };

  async function copyIfStale(source, target) {
    const stampPath = join(path.dirname(target), STAMP_FILE);
    const stamp = await readText(fs, stampPath);
    let current = false;
    try {
      current =
        JSON.parse(stamp || "{}").version === appVersion &&
        fs.existsSync(target);
    } catch {}
    if (current) return false;
    await fs.promises.mkdir(path.dirname(target), { recursive: true });
    const temporary = `${target}.${process.pid}.tmp`;
    await fs.promises.copyFile(source, temporary);
    if (platform !== "win32") await fs.promises.chmod(temporary, 0o755);
    await fs.promises.rename(temporary, target);
    await fs.promises.writeFile(
      stampPath,
      `${JSON.stringify({ version: appVersion })}\n`,
    );
    return true;
  }

  async function readPid() {
    const text = await readText(fs, paths.hostPid);
    const pid = Number.parseInt(String(text || "").trim(), 10);
    return Number.isInteger(pid) && pid > 0 && isProcessAlive(pid) ? pid : null;
  }

  async function waitFor(check, timeoutMs = 10_000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if (await check()) return true;
      await sleep(200);
    }
    return false;
  }

  async function trayRunning() {
    const result = await run("tasklist", [
      "/FI",
      `IMAGENAME eq ${TRAY_EXECUTABLE}`,
      "/NH",
      "/FO",
      "CSV",
    ]);
    return result.stdout.toLowerCase().includes(TRAY_EXECUTABLE.toLowerCase());
  }

  const kinds = {
    "systemd-user": {
      installed: () => fs.existsSync(paths.unit),
      async autostart() {
        const result = await run("systemctl", [
          "--user",
          "is-enabled",
          `${SERVICE_NAME}.service`,
        ]);
        return result.stdout.trim() === "enabled";
      },
      async running() {
        const result = await run("systemctl", [
          "--user",
          "is-active",
          `${SERVICE_NAME}.service`,
        ]);
        return result.stdout.trim() === "active";
      },
      async install({ autostart, upgrade }) {
        const copied =
          paths.linuxServer !== bundledServer
            ? await copyIfStale(bundledServer, paths.linuxServer)
            : false;
        const unitChanged = await writeIfChanged(
          fs,
          paths.unit,
          systemdUnit({ executable: paths.linuxServer, env }),
        );
        if (unitChanged) await ok("systemctl", ["--user", "daemon-reload"]);
        await this.setAutostart(autostart);
        const restart = copied || unitChanged || upgrade;
        await ok("systemctl", [
          "--user",
          restart ? "restart" : "start",
          `${SERVICE_NAME}.service`,
        ]);
        return restart;
      },
      async setAutostart(enabled) {
        await ok("systemctl", [
          "--user",
          enabled ? "enable" : "disable",
          `${SERVICE_NAME}.service`,
        ]);
      },
      start: () =>
        ok("systemctl", ["--user", "start", `${SERVICE_NAME}.service`]),
      stop: () =>
        ok("systemctl", ["--user", "stop", `${SERVICE_NAME}.service`]),
      restart: () =>
        ok("systemctl", ["--user", "restart", `${SERVICE_NAME}.service`]),
    },
    "launch-agent": {
      installed: () => fs.existsSync(paths.plist),
      async autostart() {
        return /<key>RunAtLoad<\/key>\s*<true\/>/.test(
          (await readText(fs, paths.plist)) || "",
        );
      },
      async loaded() {
        return (await run("launchctl", ["print", launchTarget])).code === 0;
      },
      async running() {
        const result = await run("launchctl", ["print", launchTarget]);
        return result.code === 0 && /state = running/.test(result.stdout);
      },
      async install({ autostart, upgrade }) {
        const before = await readText(fs, paths.plist);
        const next = launchAgentPlist({
          program,
          runAtLoad: autostart,
          logPath: paths.launchLog,
          env,
        });
        // RunAtLoad is only read at the next sign-in; any other change needs
        // launchd to load the new definition.
        const definition = (text) =>
          String(text || "").replace(
            /<key>RunAtLoad<\/key>\s*<(true|false)\/>/,
            "",
          );
        const changed = definition(before) !== definition(next);
        await writeIfChanged(fs, paths.plist, next);
        await fs.promises.mkdir(path.dirname(paths.launchLog), {
          recursive: true,
        });
        const loaded = await this.loaded();
        if (loaded && changed) {
          await run("launchctl", ["bootout", launchTarget]);
        }
        if (!loaded || changed) {
          await ok("launchctl", ["bootstrap", `gui/${uid}`, paths.plist]);
        }
        await ok("launchctl", [
          "kickstart",
          ...(upgrade && loaded && !changed ? ["-k"] : []),
          launchTarget,
        ]);
        return changed || upgrade;
      },
      async setAutostart(enabled) {
        await writeIfChanged(
          fs,
          paths.plist,
          launchAgentPlist({
            program,
            runAtLoad: enabled,
            logPath: paths.launchLog,
            env,
          }),
        );
      },
      async start() {
        if (!(await this.loaded())) {
          await ok("launchctl", ["bootstrap", `gui/${uid}`, paths.plist]);
        }
        await ok("launchctl", ["kickstart", launchTarget]);
      },
      // The host exits 0 on SIGTERM, which KeepAlive leaves stopped.
      stop: () => run("launchctl", ["kill", "SIGTERM", launchTarget]),
      restart: () => ok("launchctl", ["kickstart", "-k", launchTarget]),
    },
    "xdg-autostart": {
      installed: () => fs.existsSync(paths.autostart),
      async autostart() {
        return /^Hidden=false$/m.test(
          (await readText(fs, paths.autostart)) || "",
        );
      },
      async running() {
        return (await readPid()) !== null;
      },
      async install({ autostart, upgrade }) {
        await this.setAutostart(autostart);
        if (upgrade && (await readPid())) await this.stop();
        if (!(await readPid())) await this.start();
        return upgrade;
      },
      async setAutostart(enabled) {
        await writeIfChanged(
          fs,
          paths.autostart,
          xdgAutostartEntry({ program, enabled, env }),
        );
      },
      async start() {
        spawnDetached(program[0], program.slice(1), passthrough(env));
        await waitFor(async () => (await readPid()) !== null, 15_000);
      },
      async stop() {
        const pid = await readPid();
        if (!pid) return;
        killProcess(pid, "SIGTERM");
        await waitFor(() => !isProcessAlive(pid), 15_000);
      },
      async restart() {
        await this.stop();
        await this.start();
      },
    },
    "windows-tray": {
      installed: () => fs.existsSync(paths.tray),
      async autostart() {
        const result = await run("reg", ["query", RUN_KEY, "/v", RUN_VALUE]);
        return (
          result.code === 0 &&
          result.stdout.includes(windowsRunCommand(paths.tray))
        );
      },
      running: trayRunning,
      async install({ autostart, upgrade }) {
        const stale = await (async () => {
          const stamp = await readText(
            fs,
            join(path.dirname(paths.tray), STAMP_FILE),
          );
          try {
            return (
              JSON.parse(stamp || "{}").version !== appVersion ||
              !fs.existsSync(paths.tray)
            );
          } catch {
            return true;
          }
        })();
        if (stale && (await trayRunning())) await this.stop();
        const copied = await copyIfStale(bundledTray, paths.tray);
        await this.setAutostart(autostart);
        if (upgrade && !copied && (await trayRunning())) await this.stop();
        if (!(await trayRunning())) await this.start();
        return copied || upgrade;
      },
      async setAutostart(enabled) {
        if (enabled) {
          await ok("reg", [
            "add",
            RUN_KEY,
            "/v",
            RUN_VALUE,
            "/t",
            "REG_SZ",
            "/d",
            windowsRunCommand(paths.tray),
            "/f",
          ]);
        } else {
          await run("reg", ["delete", RUN_KEY, "/v", RUN_VALUE, "/f"]);
        }
      },
      async start() {
        spawnDetached(paths.tray, ["--background"], passthrough(env));
        await waitFor(trayRunning, 10_000);
      },
      async stop() {
        // No /F: the tray gets WM_CLOSE and shuts the server down cleanly.
        await run("taskkill", ["/IM", TRAY_EXECUTABLE]);
        await waitFor(async () => !(await trayRunning()), 15_000);
      },
      async restart() {
        await this.stop();
        await this.start();
      },
    },
  };
  const driver = kinds[kind];

  return {
    kind,
    label: KIND_LABELS[kind] ?? "",
    paths,
    dataDir,
    port: servicePort(env),
    available: Boolean(driver),
    /**
     * Installs (or refreshes) the service and makes sure it runs. `token`
     * lets this app recognise the server it installed; `upgrade` restarts a
     * running server that is older than this app.
     */
    async ensure({ token, autostart, upgrade = false }) {
      if (!driver) throw new Error("This computer has no background service.");
      // First install: launch at startup, like Roon Server. Afterwards the
      // user's choice (from Parson, the menu bar or the tray) is kept.
      const launchAtStartup =
        typeof autostart === "boolean"
          ? autostart
          : driver.installed()
            ? await driver.autostart().catch(() => true)
            : true;
      await writeIfChanged(
        fs,
        paths.hostFile,
        hostFileContents(await readText(fs, paths.hostFile), { token }),
        0o600,
      );
      const restarted = await driver.install({
        autostart: launchAtStartup,
        upgrade,
      });
      return { kind, restarted: Boolean(restarted) };
    },
    async status() {
      if (!driver) return { kind, available: false };
      const [running, autostart] = await Promise.all([
        driver.running().catch(() => false),
        driver.autostart().catch(() => false),
      ]);
      return {
        kind,
        label: KIND_LABELS[kind],
        available: true,
        installed: Boolean(driver.installed()),
        running,
        autostart,
      };
    },
    setAutostart: (enabled) => driver.setAutostart(Boolean(enabled)),
    start: () => driver.start(),
    stop: () => driver.stop(),
    restart: () => driver.restart(),
  };
}

module.exports = {
  DEFAULT_PORT,
  HOST_FILE,
  LAUNCH_AGENT_LABEL,
  RUN_KEY,
  RUN_VALUE,
  SERVICE_NAME,
  createServerService,
  hostFileContents,
  hostStateDir,
  launchAgentPlist,
  serverDataDir,
  serverHostProgram,
  serviceKind,
  servicePort,
  systemdUnit,
  windowsRunCommand,
  xdgAutostartEntry,
};
