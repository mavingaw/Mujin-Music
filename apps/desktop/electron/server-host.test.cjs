const test = require("node:test");
const assert = require("node:assert/strict");
const {
  MAX_CRASHES,
  hostMenuModel,
  restartDelay,
  shouldGiveUp,
} = require("./server-host.cjs");

const labels = (state) =>
  hostMenuModel(state)
    .filter((item) => item.type !== "separator")
    .map((item) => item.label);

test("the menu-bar item shows Running, Launch at startup and Quit", () => {
  const menu = hostMenuModel({
    status: "running",
    port: 1993,
    autostart: true,
  });
  assert.deepEqual(labels({ status: "running", port: 1993, autostart: true }), [
    "Mujin Music Server — Running on port 1993",
    "Open Mujin Music",
    "Connect a phone or another computer…",
    "Restart server",
    "Launch at startup",
    "Open logs",
    "Quit Mujin Music Server",
  ]);
  const autostart = menu.find((item) => item.action === "toggle-autostart");
  assert.equal(autostart.type, "checkbox");
  assert.equal(autostart.checked, true);
  assert.equal(menu[0].enabled, false);
});

test("a stopped server offers Start and hides phone pairing", () => {
  const menu = hostMenuModel({ status: "failed", autostart: false });
  assert.equal(
    menu[0].label,
    "Mujin Music Server — Stopped after repeated errors",
  );
  assert.ok(menu.some((item) => item.action === "start"));
  assert.ok(!menu.some((item) => item.action === "restart"));
  assert.equal(
    menu.find((item) => item.action === "open-connections").enabled,
    false,
  );
});

test("crashes back off and stop after repeated failures", () => {
  assert.equal(restartDelay(1), 1_000);
  assert.equal(restartDelay(3), 5_000);
  assert.equal(restartDelay(40), 30_000);
  const now = 10_000_000;
  const recent = Array.from(
    { length: MAX_CRASHES },
    (_, index) => now - index * 1000,
  );
  assert.equal(shouldGiveUp(recent, now), true);
  assert.equal(shouldGiveUp(recent.slice(1), now), false);
  // Old crashes do not count.
  assert.equal(
    shouldGiveUp(
      recent.map((time) => time - 10 * 60_000),
      now,
    ),
    false,
  );
});
