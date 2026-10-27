const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const {
  commandAllowed,
  emptyConfig,
  forgetServer,
  loadConfig,
  nearestExistingFolder,
  normalizeServerAddress,
  parseConfig,
  probeMessage,
  probeServer,
  reconnectDelay,
  rememberServer,
  saveConfig,
  serializeConfig,
  isThisComputer,
  pickPrimary,
  serverRows,
  startupPlan,
  useLocalServer,
  useRemoteServer,
} = require("./server-config.cjs");

const living = {
  instanceId: "living-room",
  name: "Living room",
  origin: "http://192.168.1.20:1993",
  serverVersion: "1.0.1",
};

test("server addresses accept what people type", () => {
  assert.equal(
    normalizeServerAddress("192.168.1.20"),
    "http://192.168.1.20:1993",
  );
  assert.equal(
    normalizeServerAddress(" nas.local:8080 "),
    "http://nas.local:8080",
  );
  assert.equal(
    normalizeServerAddress("https://music.example/library"),
    "https://music.example",
  );
  assert.equal(normalizeServerAddress("fd00::42"), "http://[fd00::42]:1993");
  assert.equal(normalizeServerAddress("http://user:pw@nas.local"), "");
  assert.equal(normalizeServerAddress("file:///etc/passwd"), "");
  assert.equal(normalizeServerAddress(""), "");
});

test("first run asks; an existing install keeps running its own server", () => {
  const config = emptyConfig();
  assert.deepEqual(startupPlan({ config }), {
    kind: "choose",
    reason: "first-run",
  });
  assert.deepEqual(startupPlan({ config, hasLocalLibrary: true }), {
    kind: "local",
  });
  // Once the choice was offered, quitting at the chooser asks again.
  assert.equal(
    startupPlan({ config, configExists: true, hasLocalLibrary: true }).kind,
    "choose",
  );
});

test("a saved choice decides startup", () => {
  const remote = useRemoteServer(emptyConfig(), living, 10);
  const plan = startupPlan({ config: remote, configExists: true });
  assert.equal(plan.kind, "remote");
  assert.equal(plan.server.origin, living.origin);
  assert.equal(
    startupPlan({ config: useLocalServer(remote), configExists: true }).kind,
    "local",
  );
  const orphaned = { ...remote, activeServerId: null };
  assert.deepEqual(startupPlan({ config: orphaned, configExists: true }), {
    kind: "choose",
    reason: "no-server",
  });
});

test("command line and environment override the saved choice", () => {
  const local = useLocalServer(emptyConfig());
  const forced = startupPlan({
    config: local,
    argv: ["parson", "--server=10.0.0.9"],
  });
  assert.equal(forced.kind, "remote");
  assert.equal(forced.forced, true);
  assert.equal(forced.server.origin, "http://10.0.0.9:1993");
  assert.equal(
    startupPlan({
      config: local,
      env: { PARSON_DESKTOP_SERVER: "nas.local" },
    }).server.origin,
    "http://nas.local:1993",
  );
  assert.deepEqual(
    startupPlan({ config: local, argv: ["--server", "ftp://x"] }),
    { kind: "choose", reason: "invalid-address" },
  );
  assert.equal(
    startupPlan({
      config: useRemoteServer(emptyConfig(), living),
      env: { PARSON_DESKTOP_MODE: "local" },
    }).kind,
    "local",
  );
  assert.equal(
    startupPlan({ config: local, argv: ["--choose-server"] }).kind,
    "choose",
  );
});

test("remote mode never plans to start the local server", () => {
  const config = useRemoteServer(emptyConfig(), living);
  for (const hasLocalLibrary of [true, false]) {
    const plan = startupPlan({ config, configExists: true, hasLocalLibrary });
    assert.notEqual(plan.kind, "local");
  }
});

test("saved servers follow a server that moved to a new address", () => {
  let config = useRemoteServer(emptyConfig(), living, 1);
  config = rememberServer(
    config,
    { ...living, origin: "http://192.168.1.77:1993" },
    2,
  );
  assert.equal(config.servers.length, 1);
  assert.equal(config.servers[0].origin, "http://192.168.1.77:1993");
  assert.equal(config.servers[0].lastConnectedAt, 2);
  assert.equal(config.activeServerId, "living-room");
});

test("the most recent server comes first and the list is bounded", () => {
  let config = emptyConfig();
  for (let index = 0; index < 30; index += 1) {
    config = rememberServer(config, {
      instanceId: `server-${index}`,
      name: `Server ${index}`,
      origin: `http://10.0.0.${index + 1}:1993`,
    });
  }
  assert.equal(config.servers.length, 20);
  assert.equal(config.servers[0].instanceId, "server-29");
});

test("forgetting the active server returns to the chooser", () => {
  const config = useRemoteServer(emptyConfig(), living);
  const forgotten = forgetServer(config, "living-room");
  assert.deepEqual(forgotten.servers, []);
  assert.equal(forgotten.mode, null);
  assert.equal(forgotten.activeServerId, null);
  assert.equal(
    startupPlan({ config: forgotten, configExists: true }).kind,
    "choose",
  );
});

test("config files are validated and written atomically", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "parson-servers-"));
  try {
    assert.deepEqual(loadConfig(directory), {
      exists: false,
      config: emptyConfig(),
    });
    const config = useRemoteServer(emptyConfig(), living, 5);
    saveConfig(directory, config);
    const loaded = loadConfig(directory);
    assert.equal(loaded.exists, true);
    assert.equal(loaded.config.mode, "remote");
    assert.equal(loaded.config.servers[0].origin, living.origin);
    fs.writeFileSync(path.join(directory, "servers.json"), "{not json");
    assert.deepEqual(loadConfig(directory), {
      exists: true,
      config: emptyConfig(),
    });
    assert.deepEqual(
      parseConfig({
        mode: "sideways",
        activeServerId: "ghost",
        servers: [{ origin: "javascript:alert(1)" }, living, living],
      }).servers.length,
      1,
    );
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

test("chooser rows merge saved and discovered servers", () => {
  const config = useRemoteServer(
    rememberServer(emptyConfig(), {
      instanceId: "office",
      name: "Office",
      origin: "http://10.0.0.5:1993",
    }),
    living,
  );
  const rows = serverRows({
    config,
    discovered: [
      { ...living, origin: "http://192.168.1.99:1993" },
      {
        instanceId: "this-pc",
        name: "This PC",
        origin: "http://10.0.0.2:1993",
      },
      { instanceId: "nas", name: "NAS", origin: "http://10.0.0.8:1993" },
    ],
    localInstanceId: "this-pc",
    signedIn: new Set(["http://192.168.1.99:1993"]),
  });
  assert.deepEqual(
    rows.map((row) => [row.name, row.saved, row.nearby, row.active]),
    [
      ["Living room", true, true, true],
      ["NAS", false, true, false],
      ["Office", true, false, false],
    ],
  );
  assert.equal(rows[0].origin, "http://192.168.1.99:1993");
  assert.equal(rows[0].signedIn, true);
});

test("remote servers' web apps cannot touch this computer's files", () => {
  assert.equal(commandAllowed("select_music_folder", "local"), true);
  assert.equal(commandAllowed("select_music_folder", "remote"), false);
  assert.equal(commandAllowed("show_track_in_file_manager", "remote"), false);
  assert.equal(commandAllowed("server_choose", "remote"), true);
  assert.equal(commandAllowed("updates_check", "remote"), true);
  assert.equal(commandAllowed("rm_rf", "local"), false);
});

test("reconnect attempts back off to a steady pace", () => {
  assert.deepEqual(
    [0, 1, 2, 3, 4, 5, 50].map(reconnectDelay),
    [1_000, 2_000, 3_000, 5_000, 8_000, 15_000, 15_000],
  );
});

function response(body, status = 200) {
  return {
    ok: status >= 200 && status < 300,
    status,
    text: async () => (typeof body === "string" ? body : JSON.stringify(body)),
  };
}

test("probing tells Mujin Music, other services and dead addresses apart", async () => {
  const manifest = {
    protocol: "parson",
    product: "parson-music",
    protocolVersion: 1,
    instanceId: "living-room",
    name: "Living room",
  };
  const fetchWith = (result) => async () => {
    if (result instanceof Error) throw result;
    return result;
  };
  assert.deepEqual(
    await probeServer(living.origin, { fetch: fetchWith(response(manifest)) }),
    { kind: "ok", manifest },
  );
  assert.equal(
    (
      await probeServer(living.origin, {
        fetch: fetchWith(response({ ...manifest, protocolVersion: 9 })),
      })
    ).kind,
    "incompatible",
  );
  assert.equal(
    (
      await probeServer(living.origin, {
        fetch: fetchWith(response("<html>router</html>")),
      })
    ).kind,
    "not-parson",
  );
  assert.equal(
    (await probeServer(living.origin, { fetch: fetchWith(response({}, 404)) }))
      .kind,
    "not-parson",
  );
  const down = await probeServer(living.origin, {
    fetch: fetchWith(new Error("ECONNREFUSED")),
  });
  assert.equal(down.kind, "unreachable");
  assert.match(
    probeMessage(down, living.origin),
    /No Mujin Music server answered/,
  );
  assert.match(
    probeMessage({ kind: "not-parson" }, living.origin),
    /isn’t a Mujin Music server/,
  );
});

test("a connected server is never shown as offline", () => {
  const config = useRemoteServer(emptyConfig(), living, 1_000);
  // Discovery has not confirmed it yet (or the chooser opened mid-scan),
  // but the window is connected and the monitor sees it.
  const [row] = serverRows({ config, discovered: [], activeId: "living-room" });
  assert.equal(row.active, true);
  assert.equal(row.online, true);
  // While reconnecting, the active row is honestly offline.
  const [down] = serverRows({
    config,
    discovered: [],
    activeId: "living-room",
    activeOnline: false,
  });
  assert.equal(down.active, true);
  assert.equal(down.online, false);
  // Connected at a new DHCP address: still the same active row, matched by id.
  const [moved] = serverRows({
    config,
    discovered: [{ ...living, origin: "http://192.168.1.77:1993" }],
    activeId: "living-room",
  });
  assert.equal(moved.origin, "http://192.168.1.77:1993");
  assert.equal(moved.active && moved.online && moved.nearby, true);
});

test("forgetting a server that is still online keeps it listed, signed out", () => {
  const config = useRemoteServer(emptyConfig(), living, 1_000);
  const forgotten = forgetServer(config, "living-room");
  const rows = serverRows({
    config: forgotten,
    discovered: [living],
    activeId: null,
  });
  assert.deepEqual(
    rows.map((row) => [
      row.name,
      row.saved,
      row.nearby,
      row.online,
      row.active,
    ]),
    [["Living room", false, true, true, false]],
  );
  // Gone from the network too: then it disappears.
  assert.deepEqual(serverRows({ config: forgotten, discovered: [] }), []);
});

test("this computer's own server is the local card, not a remote row", () => {
  let config = useRemoteServer(emptyConfig(), living, 1_000);
  config = rememberServer(config, {
    instanceId: "this-pc",
    name: "This PC",
    origin: "http://192.168.1.5:1993",
  });
  const rows = serverRows({
    config,
    discovered: [
      {
        instanceId: "this-pc",
        name: "This PC",
        origin: "http://192.168.1.5:1993",
      },
    ],
    localInstanceId: "this-pc",
  });
  assert.deepEqual(
    rows.map((row) => row.name),
    ["Living room"],
  );
  assert.equal(isThisComputer("this-pc", "this-pc"), true);
  assert.equal(isThisComputer("nas", "this-pc"), false);
  assert.equal(isThisComputer("", ""), false);
});

test("a library taken over by a restore lists both servers the same way on every scan", () => {
  // Connected to the old server; the new one restored its backup and took
  // over its identity, and both are switched on.
  const basement = {
    instanceId: "library-1",
    name: "Basement NAS",
    origin: "http://192.0.2.2:2014",
    serverVersion: "1.0.1",
  };
  const study = {
    instanceId: "library-1",
    name: "Study Mini",
    origin: "http://192.0.2.2:2015",
    serverVersion: "1.0.1",
    identityAdoptedAt: 1_790_000_000_000,
  };
  const config = useRemoteServer(emptyConfig(), basement, 1_000);
  const scan = (discovered) =>
    serverRows({
      config,
      discovered,
      activeId: "library-1",
      activeOrigin: basement.origin,
    }).map((row) => [row.name, row.origin, row.active, Boolean(row.note)]);
  const expected = [
    ["Basement NAS", basement.origin, true, false],
    ["Study Mini", study.origin, false, true],
  ];
  // Whichever server answered first, and even when the old one missed a
  // scan, the connected row stays the server this window is connected to.
  assert.deepEqual(scan([basement, study]), expected);
  assert.deepEqual(scan([study, basement]), expected);
  assert.deepEqual(
    scan([study]).map(([name, origin, active]) => [name, origin, active]),
    [
      ["Basement NAS", basement.origin, true],
      ["Study Mini", study.origin, false],
    ],
  );
  const note = serverRows({
    config,
    discovered: [study, basement],
    activeId: "library-1",
    activeOrigin: basement.origin,
  })[1].note;
  assert.match(note, /Took over Basement NAS/);

  // Not connected to either: the newest takeover represents the library,
  // in either order, and the old server carries the note.
  const idle = (discovered) =>
    serverRows({ config, discovered, activeId: null }).map((row) => [
      row.name,
      row.saved,
      Boolean(row.note),
    ]);
  assert.deepEqual(idle([basement, study]), [
    ["Study Mini", true, false],
    ["Basement NAS", false, true],
  ]);
  assert.deepEqual(idle([study, basement]), idle([basement, study]));
  assert.equal(
    pickPrimary([basement, study], { activeOrigin: basement.origin }).name,
    "Basement NAS",
  );
  assert.equal(pickPrimary([basement, study]).name, "Study Mini");
});

test("Locate opens the folder picker near where the music used to be", () => {
  const os = require("node:os");
  const fs = require("node:fs");
  const path = require("node:path");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "parson-locate-"));
  try {
    const disk = path.join(root, "mnt", "music-disk");
    fs.mkdirSync(disk, { recursive: true });
    assert.equal(
      nearestExistingFolder(path.join(disk, "Old Library", "FLAC")),
      disk,
    );
    assert.equal(nearestExistingFolder(disk), disk);
    assert.equal(nearestExistingFolder("relative/path"), "");
    assert.equal(nearestExistingFolder(""), "");
    // A walk over an injected file system.
    const existing = new Set(["/", "/Volumes"]);
    assert.equal(
      nearestExistingFolder("/Volumes/Old Disk/Music", {
        isDirectory: (candidate) => existing.has(candidate),
      }),
      "/Volumes",
    );
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("what a hostile server says about itself is cleaned before it is saved", () => {
  const hostile = {
    origin: "http://10.0.0.66:1993/ignored/path?x=1",
    instanceId: "../../etc/passwd\nforged",
    name: `Evil\n[forged log line]\r\u001b[31m‮redro${"x".repeat(5000)}`,
    serverVersion: "1.0.0\n<script>alert(1)</script>",
    manual: "yes",
    extra: { huge: "y".repeat(100_000) },
    __proto__: { polluted: true },
  };
  const config = useRemoteServer(emptyConfig(), hostile, 1_700_000_000_000);
  const [server] = config.servers;
  assert.deepEqual(Object.keys(server).sort(), [
    "id",
    "instanceId",
    "lastConnectedAt",
    "manual",
    "name",
    "origin",
    "serverVersion",
  ]);
  assert.equal(server.origin, "http://10.0.0.66:1993");
  assert.equal(server.instanceId, "", "a path-like id is not an id");
  assert.equal(server.id, "http://10.0.0.66:1993");
  assert.equal(server.serverVersion, "");
  assert.equal(server.manual, false);
  assert.doesNotMatch(server.name, /[\u0000-\u001f\u007f‮]/u);
  assert.ok(server.name.startsWith("Evil [forged log line]"), server.name);
  assert.ok(Array.from(server.name).length <= 128);

  const saved = serializeConfig({
    ...config,
    localInstanceId: "this-pc\n",
    servers: [...config.servers, { ...hostile, lastConnectedAt: -1 }],
  });
  assert.ok(Buffer.byteLength(saved) < 4096, "no unknown fields are written");
  assert.doesNotMatch(saved, /forged\n|polluted|huge/);
  // A version is a version: markup in one is dropped, not stored.
  for (const server of JSON.parse(saved).servers) {
    assert.doesNotMatch(server.serverVersion, /[<>]/);
  }
  assert.equal(JSON.parse(saved).localInstanceId, "this-pc");

  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "parson-hostile-"));
  try {
    saveConfig(directory, config);
    const written = fs.readFileSync(
      path.join(directory, "servers.json"),
      "utf8",
    );
    assert.equal(written, serializeConfig(config));
    assert.deepEqual(loadConfig(directory).config, parseConfig(config));
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});
