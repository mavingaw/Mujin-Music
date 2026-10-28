const { spawnSync } = require("node:child_process");
const { existsSync, readdirSync, rmSync } = require("node:fs");
const path = require("node:path");
const {
  generateThirdPartyNotices,
} = require("../../../tools/generate-third-party-notices.cjs");
const {
  latestDownloadBase,
  releasesBase,
} = require("../../../tools/releases-base.cjs");

const desktopDirectory = path.resolve(__dirname, "..");
const outputDirectory = path.resolve(
  desktopDirectory,
  "../../target/release/bundle/electron",
);
const requestedArchitecture = process.env.PARSON_BUILD_ARCH || process.arch;
const targetPlatform = process.env.PARSON_BUILD_PLATFORM || process.platform;
const architecture =
  requestedArchitecture === "x86_64" || requestedArchitecture === "amd64"
    ? "x64"
    : requestedArchitecture === "aarch64"
      ? "arm64"
      : requestedArchitecture;

generateThirdPartyNotices();

// The update feed and the release page the app links to come from
// MUJIN_RELEASES_BASE (tools/releases-base.cjs), so a build made for another
// forge checks that forge for updates. electron-updater's generic provider
// reads latest*.yml from the newest stable release's download folder.
const base = releasesBase(process.env);
const releaseArguments = [
  `--config.publish.url=${latestDownloadBase(base)}`,
  `--config.extraMetadata.parsonUpdates.releasePage=${base}/latest`,
];

if (architecture !== "x64" && architecture !== "arm64") {
  throw new Error(
    `Desktop packaging does not support ${requestedArchitecture}.`,
  );
}

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: desktopDirectory,
    stdio: "inherit",
    env: {
      ...process.env,
      ...(targetPlatform === "win32"
        ? { ELECTRON_BUILDER_7Z_FILTER: "BCJ" }
        : {}),
    },
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} exited with status ${result.status}`);
  }
}

function cleanOutput(predicate) {
  if (!existsSync(outputDirectory)) {
    return;
  }
  for (const entry of readdirSync(outputDirectory, { withFileTypes: true })) {
    if (!predicate(entry.name)) {
      continue;
    }
    rmSync(path.join(outputDirectory, entry.name), {
      recursive: true,
      force: true,
    });
  }
}

if (targetPlatform === "linux") {
  cleanOutput(
    (name) =>
      (name.startsWith("linux") && name.endsWith("-unpacked")) ||
      name.endsWith(".AppImage") ||
      name.endsWith(".deb"),
  );
  run("bash", [path.join(__dirname, "build-linux.sh"), ...releaseArguments]);
} else if (targetPlatform === "win32") {
  cleanOutput(
    (name) =>
      (name.startsWith("win") && name.endsWith("-unpacked")) ||
      (name.startsWith("Mujin-Music-Setup-") && name.endsWith(".exe")) ||
      name.endsWith("-portable.exe"),
  );
  const args = [
    require.resolve("electron-builder/cli.js"),
    "--win",
    "nsis",
    "portable",
    `--${architecture}`,
    "--publish=never",
    ...releaseArguments,
  ];
  run(process.execPath, args);
} else if (targetPlatform === "darwin") {
  cleanOutput(
    (name) =>
      (name.startsWith("mac") && !name.endsWith(".dmg")) ||
      name.endsWith(".dmg") ||
      name.endsWith(".zip"),
  );
  const args = [
    require.resolve("electron-builder/cli.js"),
    "--mac",
    "dmg",
    "zip",
    `--${architecture}`,
    "--publish=never",
    ...releaseArguments,
  ];
  if (process.env.CSC_LINK || process.env.CSC_NAME) {
    args.push(
      "--config.extraMetadata.parsonUpdates.macSigned=true",
      "--config.mac.hardenedRuntime=true",
    );
    if (process.env.APPLE_ID || process.env.APPLE_API_KEY) {
      args.push("--config.mac.notarize=true");
    }
  } else {
    args.push(
      "--config.mac.identity=null",
      "--config.mac.hardenedRuntime=false",
      "--config.mac.notarize=false",
    );
  }
  run(process.execPath, args);
} else {
  throw new Error(`Desktop packaging is not configured for ${targetPlatform}.`);
}
