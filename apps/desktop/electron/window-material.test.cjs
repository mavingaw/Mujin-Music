const test = require("node:test");
const assert = require("node:assert/strict");

const {
  applyWindowMaterial,
  windowMaterialSupport,
} = require("./window-material.cjs");

function fakeWindow() {
  const calls = [];
  return {
    calls,
    setBackgroundMaterial: (value) => calls.push(["material", value]),
    setVibrancy: (value) => calls.push(["vibrancy", value]),
    setBackgroundColor: (value) => calls.push(["color", value]),
  };
}

test("Mica and Acrylic need Windows 11 22H2; macOS offers vibrancy", () => {
  assert.deepEqual(windowMaterialSupport("win32", "10.0.22631"), [
    "mica",
    "acrylic",
  ]);
  assert.deepEqual(windowMaterialSupport("win32", "10.0.19045"), []);
  assert.deepEqual(windowMaterialSupport("darwin", "24.1.0"), ["vibrancy"]);
  assert.deepEqual(windowMaterialSupport("linux", "6.8.0"), []);
});

test("a supported material makes the window see-through behind the page", () => {
  const window = fakeWindow();
  assert.equal(
    applyWindowMaterial(window, "acrylic", ["mica", "acrylic"]),
    "acrylic",
  );
  assert.deepEqual(window.calls, [
    ["material", "acrylic"],
    ["color", "#00000000"],
  ]);
  const mac = fakeWindow();
  assert.equal(applyWindowMaterial(mac, "vibrancy", ["vibrancy"]), "vibrancy");
  assert.deepEqual(mac.calls, [
    ["vibrancy", "under-window"],
    ["color", "#00000000"],
  ]);
});

test("turning glass off, or asking for the impossible, restores an opaque window", () => {
  const window = fakeWindow();
  assert.equal(applyWindowMaterial(window, null, ["mica", "acrylic"]), null);
  assert.deepEqual(window.calls, [
    ["material", "none"],
    ["color", "#000000"],
  ]);
  const linux = fakeWindow();
  assert.equal(applyWindowMaterial(linux, "mica", []), null);
  assert.deepEqual(linux.calls, [["color", "#000000"]]);
  const broken = fakeWindow();
  broken.setBackgroundMaterial = () => {
    throw new Error("unsupported");
  };
  assert.equal(applyWindowMaterial(broken, "mica", ["mica"]), null);
  assert.deepEqual(broken.calls, [["color", "#000000"]]);
  // A failure after the material applied takes the material back off.
  const half = fakeWindow();
  half.setBackgroundColor = (value) => {
    half.calls.push(["color", value]);
    if (value === "#00000000") throw new Error("lost window");
  };
  assert.equal(applyWindowMaterial(half, "mica", ["mica", "acrylic"]), null);
  assert.deepEqual(half.calls, [
    ["material", "mica"],
    ["color", "#00000000"],
    ["material", "none"],
    ["color", "#000000"],
  ]);
});

test("an Electron without the material API stays opaque", () => {
  const old = fakeWindow();
  delete old.setBackgroundMaterial;
  assert.equal(applyWindowMaterial(old, "mica", ["mica", "acrylic"]), null);
  assert.deepEqual(old.calls, [["color", "#000000"]]);
  const mac = fakeWindow();
  delete mac.setVibrancy;
  assert.equal(applyWindowMaterial(mac, "vibrancy", ["vibrancy"]), null);
  assert.deepEqual(mac.calls, [["color", "#000000"]]);
});
