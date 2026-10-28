const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { versionFiles, versionParts, checkVersions } = require("./release.cjs");
const { generate } = require("./update-metadata.cjs");
const { render, stripComments, templatePath } = require("./release-notes.cjs");

function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "parson-release-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  return root;
}

test("synchronises packages, Expo build numbers and Rust locks without changing dependency versions", (t) => {
  const root = fixture(t);
  const put = (name, value) => {
    fs.mkdirSync(path.dirname(path.join(root, name)), { recursive: true });
    fs.writeFileSync(path.join(root, name), value);
  };
  put("package.json", '{"version":"1.0.0"}');
  put(
    "apps/mobile/package.json",
    '{"version":"1.0.0","dependencies":{"test":"9.0.0"}}',
  );
  put("apps/mobile/app.json", '{"expo":{"version":"1.0.0","ios":{}}}');
  fs.mkdirSync(path.join(root, "packages"));
  put(
    "crates/backend/Cargo.toml",
    '[package]\nname = "server"\nversion = "1.0.0"\n[dependencies]\ntest = "9"\n',
  );
  put(
    "Cargo.lock",
    '[[package]]\nname = "server"\nversion = "1.0.0"\n\n[[package]]\nname = "test"\nversion = "9.0.0"\n',
  );
  const changes = versionFiles(root, "2.3.4");
  const mobile = JSON.parse(changes.get("apps/mobile/app.json"));
  assert.equal(mobile.expo.ios.buildNumber, "2003004");
  assert.equal(mobile.expo.android.versionCode, 2003004);
  assert.equal(
    JSON.parse(changes.get("apps/mobile/package.json")).dependencies.test,
    "9.0.0",
  );
  assert.match(changes.get("Cargo.lock"), /name = "test"\nversion = "9.0.0"/);
  assert.throws(() => checkVersions(root, "2.3.4"));
  for (const [file, contents] of changes) {
    fs.writeFileSync(path.join(root, file), contents);
  }
  checkVersions(root, "2.3.4");
  for (const version of [
    "1.0",
    "1.0.1-beta",
    "01.2.3",
    "1.1000.0",
    "2001.0.0",
    "1.2.3;echo test",
  ]) {
    assert.throws(() => versionParts(version));
  }
});
test("generates complete update channels with verified hashes and correct architectures", async (t) => {
  const root = fixture(t);
  for (const name of [
    "Mujin-Music-Setup-2.3.4-x64.exe",
    "Mujin-Music-Setup-2.3.4-arm64.exe",
    "Mujin-Music-2.3.4-x64-portable.exe",
    "Mujin-Music-2.3.4-x64.zip",
    "Mujin-Music-2.3.4-arm64.zip",
    "Mujin-Music-2.3.4-x86_64.AppImage",
    "Mujin-Music-2.3.4-arm64.AppImage",
    "Mujin-Music-2.3.4-amd64.deb",
    // Another version's files are never listed.
    "Mujin-Music-2.3.40-x86_64.AppImage",
    "Mujin-Music-2.3.4.1-arm64.AppImage",
  ]) {
    fs.writeFileSync(path.join(root, name), name);
  }
  await generate(root, "2.3.4");
  const read = (file) => JSON.parse(fs.readFileSync(path.join(root, file)));
  assert.equal(read("latest.yml").files.length, 2);
  assert.equal(
    read("latest-linux-arm64.yml").files[0].url,
    "Mujin-Music-2.3.4-arm64.AppImage",
  );
  assert.equal(
    read("latest-linux.yml").files[0].url,
    "Mujin-Music-2.3.4-x86_64.AppImage",
  );
  assert.deepEqual(
    read("latest.yml").files.map((file) => file.url),
    ["Mujin-Music-Setup-2.3.4-x64.exe", "Mujin-Music-Setup-2.3.4-arm64.exe"],
  );
  assert.deepEqual(
    read("latest-mac.yml").files.map((file) => file.url),
    ["Mujin-Music-2.3.4-x64.zip", "Mujin-Music-2.3.4-arm64.zip"],
  );
  assert.match(read("latest.yml").files[0].sha512, /^[A-Za-z0-9+/]{86}==$/);
  fs.unlinkSync(path.join(root, "Mujin-Music-Setup-2.3.4-arm64.exe"));
  await assert.rejects(generate(root, "2.3.4"), /expected 2 artifacts/);

  // A release built without some packages writes only the channels it has.
  for (const name of [
    "latest.yml",
    "latest-mac.yml",
    "latest-linux-arm64.yml",
  ]) {
    fs.unlinkSync(path.join(root, name));
  }
  await generate(root, "2.3.4", { only: ["latest.yml:1", "latest-linux.yml"] });
  assert.deepEqual(
    read("latest.yml").files.map((file) => file.url),
    ["Mujin-Music-Setup-2.3.4-x64.exe"],
  );
  assert.equal(fs.existsSync(path.join(root, "latest-mac.yml")), false);
  assert.equal(fs.existsSync(path.join(root, "latest-linux-arm64.yml")), false);
  await assert.rejects(
    generate(root, "2.3.4", { only: ["latest-mac.yml:3"] }),
    /invalid artifact count/,
  );
  await assert.rejects(
    generate(root, "2.3.4", { only: ["latest-windows.yml"] }),
    /Unknown update channel/,
  );
});

test("renders the release notes with a link to each package the stable release publishes", () => {
  const template = fs.readFileSync(templatePath, "utf8");
  const certificate = "ab".repeat(32);
  const options = {
    version: "2.3.4",
    repository: "Owner/Mujin-Music",
    certificate,
  };
  const debug = render(template, { ...options, signingKey: "debug" });
  const upload = render(template, { ...options, signingKey: "upload" });

  for (const notes of [debug, upload]) {
    assert.match(notes, /^# Mujin Music 2\.3\.4\n/);
    assert.doesNotMatch(notes, /\{\{|\}\}|<!--/);
    assert.match(
      notes,
      /based on \[Parson\]\(https:\/\/github\.com\/ParsonLabs\/parson\) by\s+ParsonLabs/,
    );
    assert.match(notes, /More info\*\*, then\s+\*\*Run anyway/);
    assert.match(notes, /winget install Gyan\.FFmpeg/);
    assert.match(notes, /Install unknown apps/);
    assert.match(notes, /docker pull ghcr\.io\/owner\/mujin-music:2\.3\.4/);
    assert.ok(notes.includes(certificate));
  }
  assert.match(debug, /publicly known\s+Android debug key/);
  assert.doesNotMatch(upload, /debug key/);
  assert.match(upload, /Mujin Music release key/);

  // Every download link names a file that release.yml requires, and each
  // user-facing file it requires has a link.
  const links = [
    ...debug.matchAll(
      /https:\/\/github\.com\/Owner\/Mujin-Music\/releases\/download\/v2\.3\.4\/([^)\s]+)\)/g,
    ),
  ].map((match) => match[1]);
  assert.deepEqual([...new Set(links)].sort(), [
    "Mujin-Music-2.3.4-amd64.deb",
    "Mujin-Music-2.3.4-android-arm64-v8a.apk",
    "Mujin-Music-2.3.4-android-armeabi-v7a.apk",
    "Mujin-Music-2.3.4-arm64-portable.exe",
    "Mujin-Music-2.3.4-arm64.AppImage",
    "Mujin-Music-2.3.4-arm64.deb",
    "Mujin-Music-2.3.4-arm64.dmg",
    "Mujin-Music-2.3.4-x64-portable.exe",
    "Mujin-Music-2.3.4-x64.dmg",
    "Mujin-Music-2.3.4-x86_64.AppImage",
    "Mujin-Music-Server-2.3.4-win-x64.zip",
    "Mujin-Music-Setup-2.3.4-arm64.exe",
    "Mujin-Music-Setup-2.3.4-x64.exe",
  ]);
  const workflow = fs.readFileSync(
    path.join(__dirname, "..", ".github", "workflows", "release.yml"),
    "utf8",
  );
  const required = [...workflow.matchAll(/require_count \d+ "([^"]+)"/g)].map(
    (match) =>
      new RegExp(
        `^${match[1]
          .replaceAll("${VERSION}", "2.3.4")
          .replace(/[.+^$()|[\]\\]/g, "\\$&")
          .replaceAll("*", ".*")}$`,
      ),
  );
  for (const link of links) {
    assert.ok(
      required.some((pattern) => pattern.test(link)),
      `release.yml does not require ${link}`,
    );
  }
  for (const pattern of required) {
    if (
      /ParsonMusicServer|windows-server-update|\\\.zip\$/.test(pattern.source)
    ) {
      continue;
    }
    assert.ok(
      links.some((link) => pattern.test(link)),
      `The release notes do not link to ${pattern}`,
    );
  }

  for (const invalid of [
    { ...options, signingKey: "debug", version: "2.3" },
    { ...options, signingKey: "debug", repository: "not a repository" },
    { ...options, signingKey: "unknown" },
    { ...options, signingKey: "debug", certificate: "secret" },
  ]) {
    assert.throws(() => render(template, invalid));
  }
  assert.throws(
    () => render("{{version}} {{other}}", { ...options, signingKey: "debug" }),
    /android-signing/,
  );
});

test("renders release notes for a release published on Gitea", () => {
  const template = fs.readFileSync(templatePath, "utf8");
  const notes = render(template, {
    version: "2.3.4",
    repository: "Owner/Mujin-Music",
    certificate: "ab".repeat(32),
    signingKey: "upload",
    releasesBase: "https://git.example.test/Owner/Mujin-Music/releases",
    image: "git.example.test/owner/mujin-music",
    provenance: "checksums",
    without: ["linux-arm64"],
  });
  assert.doesNotMatch(notes, /\{\{|\}\}|<!--/);
  assert.doesNotMatch(notes, /github\.com\/Owner/);
  assert.match(
    notes,
    /\(https:\/\/git\.example\.test\/Owner\/Mujin-Music\/releases\/download\/v2\.3\.4\/Mujin-Music-Setup-2\.3\.4-x64\.exe\)/,
  );
  assert.match(notes, /src\/tag\/v2\.3\.4\/docs\/server\.md/);
  assert.match(notes, /commits\/tag\/v2\.3\.4/);
  assert.match(
    notes,
    /docker pull git\.example\.test\/owner\/mujin-music:2\.3\.4/,
  );
  assert.match(notes, /SHA256SUMS/);
  assert.doesNotMatch(notes, /arm64\.AppImage/);
  // The table stays one table without the optional row.
  assert.match(notes, /amd64\.deb\) +\|\n\| \*\*Android, 64-bit/);

  const options = {
    version: "2.3.4",
    repository: "Owner/Mujin-Music",
    certificate: "ab".repeat(32),
    signingKey: "upload",
  };
  assert.throws(() => render(template, { ...options, provenance: "signed" }));
  assert.throws(() => render(template, { ...options, without: ["macos"] }));
  assert.throws(() => render(template, { ...options, image: "Not An Image" }));
  // The copy of the notes on a LAN-only Gitea links to Gitea itself.
  assert.match(
    render(template, {
      ...options,
      releasesBase: "http://192.168.1.58:3002/Owner/Mujin-Music/releases",
    }),
    /\(http:\/\/192\.168\.1\.58:3002\/Owner\/Mujin-Music\/releases\/download\/v2\.3\.4\/Mujin-Music-Setup-2\.3\.4-x64\.exe\)/,
  );
  assert.throws(() =>
    render(template, { ...options, releasesBase: "ftp://x.test/o/r/releases" }),
  );
});

test("the Gitea release publishes the GitHub release's packages, macOS from GitHub", () => {
  const read = (file) =>
    fs.readFileSync(path.join(__dirname, "..", file), "utf8");
  const patterns = (workflow) =>
    new Set(
      [...workflow.matchAll(/require_count (?:\d+|"\$linux") "([^"]+)"/g)].map(
        (match) => match[1],
      ),
    );
  const github = patterns(read(".github/workflows/release.yml"));
  const gitea = patterns(read(".gitea/workflows/release.yml"));
  const macos = [
    "Mujin-Music-${VERSION}-*.dmg",
    "Mujin-Music-${VERSION}-*.zip",
  ];
  assert.deepEqual(
    [...github].filter((pattern) => !macos.includes(pattern)).sort(),
    [...gitea].sort(),
  );
  const macWorkflow = read(".github/workflows/macos-for-gitea.yml");
  assert.match(macWorkflow, /for ext in dmg zip dmg\.blockmap zip\.blockmap/);
  assert.match(macWorkflow, /--only latest-mac\.yml/);
  const copy = read(".gitea/workflows/macos-from-github.yml");
  assert.match(copy, /for ext in dmg zip dmg\.blockmap zip\.blockmap/);
});

test("removes nested release-note comments completely and rejects unclosed ones", () => {
  assert.equal(stripComments("a<!<!-- x -->-- hidden -->b"), "ab");
  assert.equal(stripComments("a<!-- one -->\n\nb<!-- two -->c"), "abc");
  assert.throws(() => stripComments("a<!-- never closed"), /unclosed comment/);
});
