const fs = require("node:fs");
const path = require("node:path");
const { createHash } = require("node:crypto");
const { versionParts } = require("./release.cjs");

/**
 * Update channels and the release files each one lists, named as
 * electron-builder writes them (apps/desktop/package.json artifactName):
 * Mujin-Music-Setup-<version>-<arch>.exe, Mujin-Music-<version>-<arch>.zip
 * and Mujin-Music-<version>-<x86_64|arm64>.AppImage.
 */
const channels = [
  {
    name: "latest.yml",
    file: (version) =>
      new RegExp(`^Mujin-Music-Setup-${version}-(x64|arm64)\\.exe$`),
    artifactCount: 2,
  },
  {
    name: "latest-mac.yml",
    file: (version) => new RegExp(`^Mujin-Music-${version}-(x64|arm64)\\.zip$`),
    artifactCount: 2,
  },
  {
    name: "latest-linux.yml",
    file: (version) => new RegExp(`^Mujin-Music-${version}-x86_64\\.AppImage$`),
    artifactCount: 1,
  },
  {
    name: "latest-linux-arm64.yml",
    file: (version) => new RegExp(`^Mujin-Music-${version}-arm64\\.AppImage$`),
    artifactCount: 1,
  },
];

async function sha512(file) {
  const hash = createHash("sha512");
  for await (const chunk of fs.createReadStream(file)) {
    hash.update(chunk);
  }
  return hash.digest("base64");
}

/**
 * Chooses the channels to write. Without a selection, all of them, each
 * with its usual number of files. A selection such as
 * ["latest.yml:1", "latest-linux.yml"] writes only those channels; ":<n>"
 * changes how many files a channel must list (a release built without one
 * architecture).
 */
function selectChannels(only) {
  if (!only) {
    return channels;
  }
  return only.map((entry) => {
    const [name, count] = entry.split(":");
    const channel = channels.find((candidate) => candidate.name === name);
    if (!channel) {
      throw new Error(`Unknown update channel ${name}.`);
    }
    if (count === undefined) {
      return channel;
    }
    const artifactCount = Number(count);
    if (
      !Number.isInteger(artifactCount) ||
      artifactCount < 1 ||
      artifactCount > channel.artifactCount
    ) {
      throw new Error(`${name}: invalid artifact count ${count}.`);
    }
    return { ...channel, artifactCount };
  });
}

async function generate(directory, version, { only } = {}) {
  versionParts(version);
  const names = fs.readdirSync(directory);
  // versionParts accepts digits and dots only; dots must match literally.
  const escaped = version.replaceAll(".", "\\.");

  for (const channel of selectChannels(only)) {
    const pattern = channel.file(escaped);
    const assets = names
      .filter((name) => pattern.test(name))
      .sort(
        (left, right) =>
          Number(left.includes("arm64")) - Number(right.includes("arm64")),
      );

    if (assets.length !== channel.artifactCount) {
      throw new Error(
        `${channel.name}: expected ${channel.artifactCount} artifacts, found ${assets.length}`,
      );
    }

    const files = await Promise.all(
      assets.map(async (name) => {
        const file = path.join(directory, name);
        return {
          url: name,
          size: fs.statSync(file).size,
          sha512: await sha512(file),
        };
      }),
    );
    const manifest = {
      version,
      files,
      path: files[0].url,
      sha512: files[0].sha512,
      releaseDate: new Date().toISOString(),
    };

    fs.writeFileSync(
      path.join(directory, channel.name),
      `${JSON.stringify(manifest, null, 2)}\n`,
    );
  }
}

if (require.main === module) {
  // node tools/update-metadata.cjs <directory> <version> [--only a,b:1]
  const [directory, version, flag, selection] = process.argv.slice(2);
  Promise.resolve()
    .then(() => {
      if (flag !== undefined && (flag !== "--only" || !selection)) {
        throw new Error(
          "Usage: update-metadata.cjs <dir> <version> [--only <channels>]",
        );
      }
      return generate(path.resolve(directory), version, {
        only: selection?.split(","),
      });
    })
    .catch((error) => {
      console.error(error.message);
      process.exitCode = 1;
    });
}

module.exports = { generate };
