const test = require("node:test");
const assert = require("node:assert/strict");
const { createServerMonitor } = require("./server-monitor.cjs");

const ok = (bootId = "boot-a", instanceId = "living-room") => ({
  kind: "ok",
  manifest: { instanceId, bootId, name: "Living room" },
});
const down = { kind: "unreachable", message: "ECONNREFUSED" };

function harness(results, options = {}) {
  const events = [];
  const delays = [];
  const queue = [...results];
  const monitor = createServerMonitor({
    probe: async () => queue.shift() ?? ok(),
    instanceId: "living-room",
    bootId: "boot-a",
    schedule: (_callback, delay) => {
      delays.push(delay);
      return { delay };
    },
    cancel: () => {},
    onDown: (info) => events.push(["down", info.reason]),
    onUp: (info) => events.push(["up", info.restarted]),
    onRestart: () => events.push(["restart"]),
    onReplaced: (manifest) => events.push(["replaced", manifest.instanceId]),
    onAttempt: ({ attempt }) => events.push(["attempt", attempt]),
    ...options,
  });
  monitor.start();
  return { monitor, events, delays };
}

test("a single missed probe does not flash the reconnect screen", async () => {
  const { monitor, events, delays } = harness([down, ok()]);
  assert.equal(await monitor.check(), "up");
  assert.equal(await monitor.check(), "up");
  assert.deepEqual(events, []);
  assert.equal(delays.at(-2), 1_000);
});

test("the server going away and coming back reconnects cleanly", async () => {
  const { monitor, events } = harness([down, down, down, ok("boot-b")]);
  await monitor.check();
  assert.equal(await monitor.check(), "down");
  assert.equal(await monitor.check(), "down");
  assert.equal(await monitor.check(), "up");
  assert.deepEqual(events, [
    ["down", "unreachable"],
    ["attempt", 0],
    ["attempt", 1],
    ["up", true],
  ]);
});

test("a quick restart between probes is reported without going down", async () => {
  const { monitor, events } = harness([
    ok("boot-a"),
    ok("boot-b"),
    ok("boot-b"),
  ]);
  await monitor.check();
  await monitor.check();
  await monitor.check();
  assert.deepEqual(events, [["restart"]]);
});

test("a different library at the same address stops the monitor", async () => {
  const { monitor, events } = harness([ok("boot-a", "someone-else")]);
  assert.equal(await monitor.check(), "replaced");
  assert.deepEqual(events, [["replaced", "someone-else"]]);
  // Stopped: further checks do nothing.
  assert.equal(await monitor.check(), "replaced");
});

test("starting while the server is offline waits for it", async () => {
  const { monitor, events, delays } = harness([down, ok()], {
    initiallyDown: true,
  });
  assert.equal(delays[0], 1_000);
  assert.equal(await monitor.check(), "down");
  assert.equal(await monitor.check(), "up");
  assert.deepEqual(events, [
    ["attempt", 0],
    ["up", false],
  ]);
});

test("a stopped monitor ignores late probe results", async () => {
  let release;
  const events = [];
  const monitor = createServerMonitor({
    probe: () => new Promise((resolve) => (release = resolve)),
    schedule: () => ({}),
    cancel: () => {},
    failuresBeforeDown: 1,
    onDown: () => events.push("down"),
  });
  monitor.start();
  const pending = monitor.check();
  monitor.stop();
  release(down);
  await pending;
  assert.deepEqual(events, []);
});
