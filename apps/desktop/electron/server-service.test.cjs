const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const {
  createServerService,
  hostFileContents,
  launchAgentPlist,
  serverDataDir,
  serverHostProgram,
  serviceKind,
  systemdUnit,
  windowsRunCommand,
  xdgAutostartEntry,
} = require("./server-service.cjs");

function sandbox(t) {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "parson-service-"));
  t.after(() => fs.rmSync(home, { recursive: true, force: true }));
  const bundled = path.join(home, "app", "parson-music-server");
  fs.mkdirSync(path.dirname(bundled), { recursive: true });
  fs.writeFileSync(bundled, "server-binary");
  return { home, bundled };
}

/**
 * A file's text and mode, read through one open handle so both describe the
 * same file (no check-then-use race between a stat and a read).
 */
function readWithMode(file) {
  const handle = fs.openSync(file, "r");
  try {
    return {
      text: fs.readFileSync(handle, "utf8"),
      mode: fs.fstatSync(handle).mode,
    };
  } finally {
    fs.closeSync(handle);
  }
}

/** A fake command runner that records calls and answers from a script. */
function fakeRunner(answers = {}) {
  const calls = [];
  const run = async (command, args) => {
    const line = [command, ...args].join(" ");
    calls.push(line);
    for (const [pattern, answer] of Object.entries(answers)) {
      if (line.includes(pattern)) {
        return typeof answer === "function" ? answer(line) : answer;
      }
    }
    return { code: 0, stdout: "", stderr: "" };
  };
  return { run, calls };
}

test("each platform gets its native background mechanism", () => {
  assert.equal(
    serviceKind({ platform: "win32", trayAvailable: true }),
    "windows-tray",
  );
  assert.equal(serviceKind({ platform: "win32", trayAvailable: false }), null);
  assert.equal(serviceKind({ platform: "darwin" }), "launch-agent");
  assert.equal(
    serviceKind({ platform: "linux", systemdUser: true }),
    "systemd-user",
  );
  assert.equal(
    serviceKind({ platform: "linux", systemdUser: false }),
    "xdg-autostart",
  );
});

test("the service uses the same data folder as the desktop's old sidecar", () => {
  assert.equal(
    serverDataDir({ platform: "linux", home: "/home/ada" }),
    "/home/ada/.local/share/Parson",
  );
  assert.equal(
    serverDataDir({
      platform: "linux",
      home: "/home/ada",
      env: { XDG_DATA_HOME: "/data" },
    }),
    "/data/Parson",
  );
  assert.equal(
    serverDataDir({ platform: "darwin", home: "/Users/ada" }),
    "/Users/ada/Library/Application Support/Parson",
  );
  assert.equal(
    serverDataDir({
      platform: "win32",
      home: "C:\\Users\\ada",
      env: { LOCALAPPDATA: "C:\\Users\\ada\\AppData\\Local" },
    }),
    "C:\\Users\\ada\\AppData\\Local\\Parson",
  );
});

test("server.env keeps user lines, replaces the key and respects audio choices", () => {
  const first = hostFileContents(null, { token: "a".repeat(64) });
  assert.match(first, /^PARSON_LOCAL_AUDIO=on$/m);
  assert.match(
    first,
    new RegExp(`^PARSON_DESKTOP_INSTANCE_TOKEN=${"a".repeat(64)}$`, "m"),
  );
  const edited = `${first}PARSON_LIBRARY_NAME=Living room\n`.replace(
    "PARSON_LOCAL_AUDIO=on",
    "PARSON_LOCAL_AUDIO=off",
  );
  const next = hostFileContents(edited, { token: "b".repeat(64) });
  assert.match(next, /^PARSON_LOCAL_AUDIO=off$/m);
  assert.match(next, /^PARSON_LIBRARY_NAME=Living room$/m);
  assert.equal(next.match(/PARSON_DESKTOP_INSTANCE_TOKEN/g).length, 1);
  assert.match(next, new RegExp(`TOKEN=${"b".repeat(64)}`));
  assert.equal(next.match(/^# Written by Mujin Music Desktop/gm).length, 1);
  // A file written before the rename gets the new header, not a second one.
  const legacy = hostFileContents(
    "# Written by Parson Desktop for the background server.\nPARSON_LOCAL_AUDIO=on\n",
    { token: "c".repeat(64) },
  );
  assert.equal(legacy.match(/^# Written by/gm).length, 1);
  assert.match(legacy, /^# Written by Mujin Music Desktop/m);
});

test("the systemd unit mirrors the packaged unit for a user session", () => {
  const unit = systemdUnit({
    executable: "/opt/Parson Music/parson-music-server",
    env: {},
  });
  assert.match(unit, /^ExecStart="\/opt\/Parson Music\/parson-music-server"$/m);
  assert.match(unit, /^Environment="PARSON_BIND_ADDRESS=0\.0\.0\.0"$/m);
  assert.match(unit, /^Restart=on-failure$/m);
  assert.match(unit, /^WantedBy=default\.target$/m);
  assert.doesNotMatch(unit, /DynamicUser/);
  const packaged = fs.readFileSync(
    path.resolve(
      __dirname,
      "../../../crates/backend/packaging/parson-music.service",
    ),
    "utf8",
  );
  for (const line of [
    "Description=Mujin Music server",
    "After=network-online.target sound.target",
    "RestartSec=3",
  ]) {
    assert.ok(packaged.includes(line), line);
    assert.ok(unit.includes(line), line);
  }
});

test("the LaunchAgent restarts crashes but stays stopped after Quit", () => {
  const plist = launchAgentPlist({
    program: [
      "/Applications/Parson.app/Contents/MacOS/Parson",
      "--server-host",
    ],
    runAtLoad: true,
    logPath: "/Users/ada/Library/Logs/Parson/server-host.log",
  });
  assert.match(plist, /<string>com\.parsonlabs\.parson\.server<\/string>/);
  assert.match(plist, /<key>RunAtLoad<\/key>\s*<true\/>/);
  assert.match(
    plist,
    /<key>KeepAlive<\/key>\s*<dict>\s*<key>SuccessfulExit<\/key>\s*<false\/>/,
  );
  assert.match(plist, /<string>--server-host<\/string>/);
  // Audio work must not be throttled like a background job.
  assert.match(plist, /<string>Interactive<\/string>/);
  assert.match(
    launchAgentPlist({ program: ["/a&b"], runAtLoad: false, logPath: "/l" }),
    /<string>\/a&amp;b<\/string>[\s\S]*<key>RunAtLoad<\/key>\s*<false\/>/,
  );
});

test("the server host runs from a stable path", () => {
  assert.deepEqual(
    serverHostProgram({
      execPath: "/tmp/.mount_x/parson",
      packaged: true,
      env: { APPIMAGE: "/home/ada/Applications/Parson.AppImage" },
    }),
    ["/home/ada/Applications/Parson.AppImage", "--server-host"],
  );
  assert.deepEqual(
    serverHostProgram({ execPath: "/usr/bin/electron", appPath: "/src/app" }),
    ["/usr/bin/electron", "/src/app", "--server-host"],
  );
  assert.equal(
    xdgAutostartEntry({
      program: ["/home/ada/My Apps/Parson.AppImage", "--server-host"],
      enabled: false,
    }).match(/^Exec=.*$/m)[0],
    'Exec="/home/ada/My Apps/Parson.AppImage" --server-host',
  );
  assert.equal(
    windowsRunCommand(
      "C:\\Users\\ada\\AppData\\Local\\Programs\\Parson Server\\ParsonMusicServer.exe",
    ),
    '"C:\\Users\\ada\\AppData\\Local\\Programs\\Parson Server\\ParsonMusicServer.exe" --background',
  );
});

test("systemd: first install enables, starts and leaves the key for the server", async (t) => {
  const { home, bundled } = sandbox(t);
  let enabled = false;
  const { run, calls } = fakeRunner({
    "is-enabled": () => ({
      code: 0,
      stdout: enabled ? "enabled\n" : "disabled\n",
    }),
    "--user enable": () => {
      enabled = true;
      return { code: 0, stdout: "" };
    },
    "is-active": { code: 0, stdout: "active\n" },
  });
  const service = createServerService({
    platform: "linux",
    home,
    kind: "systemd-user",
    execPath: "/usr/bin/parson",
    packaged: true,
    appVersion: "1.2.0",
    bundledServer: bundled,
    run,
  });
  const token = "c".repeat(64);
  await service.ensure({ token });
  assert.deepEqual(calls, [
    "systemctl --user daemon-reload",
    "systemctl --user enable parson-server.service",
    "systemctl --user restart parson-server.service",
  ]);
  const unit = fs.readFileSync(service.paths.unit, "utf8");
  assert.match(unit, new RegExp(`ExecStart="${bundled}"`));
  const hostFile = path.join(home, ".local/share/Parson/server.env");
  const host = readWithMode(hostFile);
  assert.match(host.text, new RegExp(token));
  assert.equal(host.mode & 0o777, 0o600);
  assert.deepEqual(await service.status(), {
    kind: "systemd-user",
    label: "systemd user service",
    available: true,
    installed: true,
    running: true,
    autostart: true,
  });

  // The user turns "Start when you sign in" off; reopening the app keeps it
  // off and does not restart a server that is already current.
  await service.setAutostart(false);
  enabled = false;
  calls.length = 0;
  await service.ensure({ token });
  assert.deepEqual(calls, [
    "systemctl --user is-enabled parson-server.service",
    "systemctl --user disable parson-server.service",
    "systemctl --user start parson-server.service",
  ]);
});

test("systemd from an AppImage runs a copy that outlives the mount", async (t) => {
  const { home, bundled } = sandbox(t);
  const { run, calls } = fakeRunner();
  const options = {
    platform: "linux",
    home,
    kind: "systemd-user",
    execPath: "/tmp/.mount_abc/parson",
    packaged: true,
    env: { APPIMAGE: path.join(home, "Parson.AppImage") },
    bundledServer: bundled,
    run,
  };
  const service = createServerService({ ...options, appVersion: "1.0.0" });
  await service.ensure({ token: "d".repeat(64), autostart: true });
  const copy = path.join(
    home,
    ".local/share/parson-server/parson-music-server",
  );
  const first = readWithMode(copy);
  assert.equal(first.text, "server-binary");
  assert.equal(first.mode & 0o111, 0o111);
  assert.match(
    fs.readFileSync(service.paths.unit, "utf8"),
    /parson-server\/parson-music-server/,
  );

  // Same version: nothing copied, the running server is left alone.
  calls.length = 0;
  await createServerService({ ...options, appVersion: "1.0.0" }).ensure({
    token: "d".repeat(64),
    autostart: true,
  });
  assert.ok(calls.includes("systemctl --user start parson-server.service"));
  assert.ok(!calls.some((call) => call.includes("restart")));

  // An update replaces the copy and restarts the service once.
  fs.writeFileSync(bundled, "server-binary-v2");
  calls.length = 0;
  const updated = createServerService({ ...options, appVersion: "1.1.0" });
  const result = await updated.ensure({
    token: "d".repeat(64),
    autostart: true,
  });
  assert.equal(result.restarted, true);
  const replaced = readWithMode(copy);
  assert.equal(replaced.text, "server-binary-v2");
  assert.equal(replaced.mode & 0o111, 0o111);
  assert.ok(calls.includes("systemctl --user restart parson-server.service"));
});

test("launchd: installs the agent once and keeps it running", async (t) => {
  const { home, bundled } = sandbox(t);
  let loaded = false;
  const { run, calls } = fakeRunner({
    "launchctl print": () =>
      loaded
        ? { code: 0, stdout: "state = running\n" }
        : { code: 113, stdout: "", stderr: "Could not find service" },
    "launchctl bootstrap": () => {
      loaded = true;
      return { code: 0, stdout: "" };
    },
  });
  const service = createServerService({
    platform: "darwin",
    home,
    uid: 501,
    kind: "launch-agent",
    execPath: "/Applications/Parson.app/Contents/MacOS/Parson",
    packaged: true,
    bundledServer: bundled,
    logsDir: path.join(home, "Library/Logs/Parson"),
    run,
  });
  await service.ensure({ token: "e".repeat(64) });
  assert.deepEqual(calls, [
    "launchctl print gui/501/com.parsonlabs.parson.server",
    `launchctl bootstrap gui/501 ${service.paths.plist}`,
    "launchctl kickstart gui/501/com.parsonlabs.parson.server",
  ]);
  assert.match(fs.readFileSync(service.paths.plist, "utf8"), /<true\/>/);

  // Turning Launch at startup off only rewrites RunAtLoad: no reload, so
  // the running server is not interrupted.
  calls.length = 0;
  await service.setAutostart(false);
  assert.deepEqual(calls, []);
  calls.length = 0;
  await service.ensure({ token: "e".repeat(64) });
  assert.deepEqual(calls, [
    "launchctl print gui/501/com.parsonlabs.parson.server",
    "launchctl kickstart gui/501/com.parsonlabs.parson.server",
  ]);
  assert.equal((await service.status()).autostart, false);

  // A newer app restarts the older server it finds running.
  calls.length = 0;
  await service.ensure({ token: "e".repeat(64), upgrade: true });
  assert.ok(
    calls.includes(
      "launchctl kickstart -k gui/501/com.parsonlabs.parson.server",
    ),
  );
});

test("Windows: copies the tray host, registers it at sign-in and starts it", async (t) => {
  const { home, bundled } = sandbox(t);
  const tray = path.join(home, "app", "ParsonMusicServer.exe");
  fs.writeFileSync(tray, "tray-v1");
  let running = false;
  const spawned = [];
  const { run, calls } = fakeRunner({
    tasklist: () => ({
      code: 0,
      stdout: running ? '"ParsonMusicServer.exe","4242"' : "INFO: No tasks",
    }),
    taskkill: () => {
      running = false;
      return { code: 0, stdout: "" };
    },
  });
  const local = path.join(home, "AppData", "Local");
  const service = createServerService({
    platform: "win32",
    home,
    env: { LOCALAPPDATA: local },
    kind: "windows-tray",
    packaged: true,
    appVersion: "1.0.0",
    bundledServer: bundled,
    bundledTray: tray,
    run,
    spawnDetached: (command, args) => {
      spawned.push([command, ...args].join(" "));
      running = true;
    },
    sleep: async () => {},
  });
  await service.ensure({ token: "f".repeat(64) });
  const installed = path.join(
    local,
    "Programs",
    "Mujin Music Server",
    "ParsonMusicServer.exe",
  );
  assert.equal(service.paths.tray, installed);
  assert.equal(fs.readFileSync(installed, "utf8"), "tray-v1");
  assert.ok(
    calls.some(
      (call) =>
        call.startsWith(
          "reg add HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run /v ParsonMusicServer",
        ) && call.includes(`"${installed}" --background`),
    ),
  );
  assert.deepEqual(spawned, [`${installed} --background`]);

  // An update: the running tray is closed gracefully (no /F), replaced and
  // started again.
  fs.writeFileSync(tray, "tray-v2");
  calls.length = 0;
  spawned.length = 0;
  const updated = createServerService({
    platform: "win32",
    home,
    env: { LOCALAPPDATA: local },
    kind: "windows-tray",
    packaged: true,
    appVersion: "1.1.0",
    bundledServer: bundled,
    bundledTray: tray,
    run,
    spawnDetached: (command, args) => {
      spawned.push([command, ...args].join(" "));
      running = true;
    },
    sleep: async () => {},
  });
  await updated.ensure({ token: "f".repeat(64), autostart: true });
  assert.ok(calls.includes("taskkill /IM ParsonMusicServer.exe"));
  assert.ok(
    !calls.some((call) => call.startsWith("taskkill") && call.includes("/F")),
  );
  assert.equal(fs.readFileSync(installed, "utf8"), "tray-v2");
  assert.equal(spawned.length, 1);
});

test("XDG fallback starts a detached server host and stops it by pid", async (t) => {
  const { home, bundled } = sandbox(t);
  const spawned = [];
  const killed = [];
  let alive = false;
  const pidFile = path.join(home, ".config/Parson Server/server-host.pid");
  const service = createServerService({
    platform: "linux",
    home,
    kind: "xdg-autostart",
    execPath: "/usr/lib/parson/parson",
    packaged: true,
    bundledServer: bundled,
    run: fakeRunner().run,
    spawnDetached: (command, args) => {
      spawned.push([command, ...args]);
      fs.mkdirSync(path.dirname(pidFile), { recursive: true });
      fs.writeFileSync(pidFile, "777\n");
      alive = true;
    },
    isProcessAlive: (pid) => pid === 777 && alive,
    killProcess: (pid, signal) => {
      killed.push([pid, signal]);
      alive = false;
    },
    sleep: async () => {},
  });
  await service.ensure({ token: "1".repeat(64) });
  assert.deepEqual(spawned, [["/usr/lib/parson/parson", "--server-host"]]);
  assert.match(
    fs.readFileSync(service.paths.autostart, "utf8"),
    /^Hidden=false$/m,
  );
  assert.equal((await service.status()).running, true);
  // Already running: opening the app again starts nothing new.
  await service.ensure({ token: "1".repeat(64) });
  assert.equal(spawned.length, 1);
  await service.stop();
  assert.deepEqual(killed, [[777, "SIGTERM"]]);
  assert.equal((await service.status()).running, false);
});

test("a failed command explains itself", async (t) => {
  const { home, bundled } = sandbox(t);
  const service = createServerService({
    platform: "linux",
    home,
    kind: "systemd-user",
    bundledServer: bundled,
    run: fakeRunner({
      "--user restart": {
        code: 1,
        stdout: "",
        stderr: "Failed to connect to bus: No medium found",
      },
    }).run,
  });
  await assert.rejects(
    service.ensure({ token: "2".repeat(64), autostart: false }),
    /restart parson-server\.service failed: Failed to connect to bus/,
  );
});
