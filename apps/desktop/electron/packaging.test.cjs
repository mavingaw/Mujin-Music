const test = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const {
  configureBuildCommand,
  createYargs,
} = require("electron-builder/out/builder");

const buildScript = path.resolve(__dirname, "../scripts/build.cjs");

function packagingArguments(platform, architecture, environment = {}) {
  let arguments;
  let buildEnvironment;
  const mockedRequire = (name) => {
    if (name === "node:child_process") {
      return {
        spawnSync: (command, args, options) => {
          assert.equal(command, process.execPath);
          arguments = args.slice(1);
          buildEnvironment = options.env;
          return { status: 0 };
        },
      };
    }
    if (name === "node:fs") {
      return { existsSync: () => false };
    }
    if (name === "../../../tools/generate-third-party-notices.cjs") {
      return { generateThirdPartyNotices: () => {} };
    }
    return require(name);
  };
  mockedRequire.resolve = require.resolve;

  vm.runInNewContext(readFileSync(buildScript, "utf8"), {
    require: mockedRequire,
    __dirname: path.dirname(buildScript),
    process: {
      execPath: process.execPath,
      env: {
        PARSON_BUILD_PLATFORM: platform,
        PARSON_BUILD_ARCH: architecture,
        ...environment,
      },
    },
  });

  const options = createYargs()
    .command(["build", "*"], "Build", configureBuildCommand)
    .exitProcess(false)
    .strict()
    .parseSync(arguments);
  return { ...options, buildEnvironment };
}

for (const architecture of ["x64", "arm64"]) {
  test(`Windows ${architecture} packaging selects both installer targets`, () => {
    const options = packagingArguments("win32", architecture);
    assert.deepEqual(options.win, ["nsis", "portable"]);
    assert.equal(options.buildEnvironment.ELECTRON_BUILDER_7Z_FILTER, "BCJ");
    assert.equal(options[architecture], true);
    assert.equal(options.publish, "never");
  });

  test(`macOS ${architecture} packaging selects both distribution targets`, () => {
    const options = packagingArguments("darwin", architecture);
    assert.deepEqual(options.mac, ["dmg", "zip"]);
    assert.equal(options[architecture], true);
    assert.equal(options.publish, "never");
  });
}

test("the update feed follows MUJIN_RELEASES_BASE", () => {
  for (const platform of ["win32", "darwin"]) {
    const github = packagingArguments(platform, "x64");
    assert.equal(
      github.config.publish.url,
      "https://github.com/mavingaw/Mujin-Music/releases/latest/download",
    );
    assert.equal(
      github.config.extraMetadata.parsonUpdates.releasePage,
      "https://github.com/mavingaw/Mujin-Music/releases/latest",
    );
    const gitea = packagingArguments(platform, "arm64", {
      MUJIN_RELEASES_BASE:
        "https://git.example.test/owner/Mujin-Music/releases",
    });
    assert.equal(
      gitea.config.publish.url,
      "https://git.example.test/owner/Mujin-Music/releases/download/latest",
    );
    assert.equal(
      gitea.config.extraMetadata.parsonUpdates.releasePage,
      "https://git.example.test/owner/Mujin-Music/releases/latest",
    );
  }
  assert.throws(() =>
    packagingArguments("win32", "x64", {
      MUJIN_RELEASES_BASE:
        "http://192.168.1.58:3002/owner/Mujin-Music/releases",
    }),
  );
  // electron-updater's generic provider reads latest*.yml from that folder.
  const metadata = require("../package.json");
  assert.equal(metadata.build.publish.provider, "generic");
});

test("every shipped desktop file is packaged", () => {
  const { readdirSync } = require("node:fs");
  const metadata = require("../package.json");
  const shipped = readdirSync(__dirname)
    .filter((name) => /\.(cjs|html)$/.test(name))
    .filter((name) => !name.endsWith(".test.cjs"))
    .map((name) => `electron/${name}`);
  for (const file of shipped) {
    assert.ok(
      metadata.build.files.includes(file),
      `${file} must be listed in apps/desktop/package.json build.files`,
    );
  }
});
