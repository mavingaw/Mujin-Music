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

test("probing tells Parson, other services and dead addresses apart", async () => {
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
  assert.match(probeMessage(down, living.origin), /No Parson server answered/);
  assert.match(
    probeMessage({ kind: "not-parson" }, living.origin),
    /isn’t a Parson server/,
  );
});
