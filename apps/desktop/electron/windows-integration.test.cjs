// Windows packaging that only runs on Windows (the NSIS firewall rules, the
// AppUserModelID behind the media overlay) is checked here against the
// sources it must agree with, so a rename on one side fails on any OS.
const test = require("node:test");
const assert = require("node:assert/strict");
const { existsSync, readFileSync } = require("node:fs");
const path = require("node:path");
const { createServerService } = require("./server-service.cjs");

const desktopDirectory = path.resolve(__dirname, "..");
const metadata = require("../package.json");

function installerScript() {
  const include = metadata.build.nsis.include;
  assert.ok(include, "build.nsis.include must name the installer script");
  const file = path.join(desktopDirectory, include);
  assert.ok(existsSync(file), `${include} must exist`);
  return readFileSync(file, "utf8");
}

function nsisDefine(script, name) {
  const match = script.match(new RegExp(`^!define ${name} "([^"]*)"$`, "m"));
  assert.ok(match, `${name} must be defined`);
  return match[1];
}

test("the installer adds and removes firewall rules", () => {
  const script = installerScript();
  assert.match(script, /^!macro customInstall$/m);
  assert.match(script, /^!macro customUnInstall$/m);
  const allow = script.match(/advfirewall firewall add rule [^\n`]*/g) ?? [];
  assert.ok(allow.length > 0, "the installer must add a rule");
  for (const command of allow) {
    assert.match(command, / dir=in /);
    assert.match(command, / action=allow /);
    assert.match(command, / profile=private /);
    assert.match(command, / program="/);
  }
  // Deletes are scoped to this install's program path, so another
  // account's install of the same rule name keeps its rule.
  const remove = script.match(/advfirewall firewall delete rule [^\n`]*/g);
  for (const command of remove ?? []) {
    assert.match(command, / program="/);
  }
  // An update must not remove the rules the new version keeps.
  const uninstall = script.slice(script.indexOf("!macro customUnInstall"));
  assert.match(uninstall, /\$\{ifNot\} \$\{isUpdated\}/);
  // electron-builder's isUpdated test writes $R9.
  const code = script
    .split("\n")
    .filter((line) => !line.trimStart().startsWith(";"))
    .join("\n");
  assert.doesNotMatch(code, /\$R9/);
});

test("the firewall rule names the background server's real location", () => {
  const script = installerScript();
  const local = path.join(path.sep, "users", "someone", "AppData", "Local");
  const service = createServerService({
    platform: "win32",
    home: path.join(path.sep, "users", "someone"),
    env: { LOCALAPPDATA: local },
    kind: "windows-tray",
    bundledServer: "parson-music-server.exe",
    bundledTray: "ParsonMusicServer.exe",
    run: async () => ({ code: 0, stdout: "", stderr: "" }),
    spawnDetached: () => {},
  });
  const relative = path
    .relative(local, service.paths.tray)
    .split(path.sep)
    .join("\\");
  assert.equal(nsisDefine(script, "MUJIN_FIREWALL_SERVER_PROGRAM"), relative);
});

test("the firewall rule names the desktop executable", () => {
  const script = installerScript();
  // APP_EXECUTABLE_FILENAME is electron-builder's `${productName}.exe`.
  assert.match(script, /"\$INSTDIR\\\$\{APP_EXECUTABLE_FILENAME\}"/);
  assert.equal(metadata.build.win.executableName, metadata.build.productName);
});

test("the server-only package ships its firewall scripts", () => {
  const serverCrate = path.resolve(
    desktopDirectory,
    "../../crates/windows-server",
  );
  const packageScript = readFileSync(
    path.join(serverCrate, "package.ps1"),
    "utf8",
  );
  for (const name of [
    "firewall.ps1",
    "Allow through Windows Firewall.cmd",
    "Remove from Windows Firewall.cmd",
  ]) {
    assert.ok(existsSync(path.join(serverCrate, name)), `${name} must exist`);
    assert.ok(packageScript.includes(`"${name}"`), `package.ps1 ships ${name}`);
  }
  const firewall = readFileSync(path.join(serverCrate, "firewall.ps1"), "utf8");
  assert.match(firewall, /\$ruleName = "Mujin Music Server"/);
  assert.match(firewall, /profile=private/);
  assert.match(firewall, /dir=in action=allow/);
  for (const name of [
    "Allow through Windows Firewall.cmd",
    "Remove from Windows Firewall.cmd",
  ]) {
    const wrapper = readFileSync(path.join(serverCrate, name), "utf8");
    assert.match(wrapper, /-File "%~dp0firewall\.ps1"/);
    assert.match(wrapper, / -Pause\r?\n$/);
  }
});

test("Windows media controls show Mujin Music, not an unknown app", () => {
  const main = readFileSync(path.join(__dirname, "main.cjs"), "utf8");
  const match = main.match(/const WINDOWS_APP_USER_MODEL_ID = "([^"]+)";/);
  assert.ok(match, "main.cjs must set an AppUserModelID on Windows");
  assert.equal(match[1], metadata.build.appId);
  assert.match(
    main,
    /if \(process\.platform === "win32"\) \{\n\s+app\.setAppUserModelId\(WINDOWS_APP_USER_MODEL_ID\);/,
  );
});
