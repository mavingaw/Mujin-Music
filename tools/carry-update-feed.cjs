const fs = require("node:fs");
const { versionParts } = require("./release.cjs");

/**
 * Carries an update feed (latest-mac.yml, latest-linux-arm64.yml) over from
 * the previous release into a release that does not have those packages
 * yet, or at all.
 *
 * electron-updater resolves each file of a feed against the folder it read
 * the feed from, so the previous feed is rewritten with absolute links to
 * the previous release's files. Its version stays the previous version:
 * clients already on it see no update, and older clients update to it, until
 * the packages arrive and replace the carried feed (macos-for-gitea.yml).
 */
function carry(feed, downloadBase) {
  const base = new URL(
    downloadBase.endsWith("/") ? downloadBase : `${downloadBase}/`,
  );
  if (base.protocol !== "https:") {
    throw new Error("The previous release must be served over HTTPS.");
  }
  if (!feed || typeof feed !== "object" || !Array.isArray(feed.files)) {
    throw new Error("The previous update feed has no files.");
  }
  versionParts(feed.version);
  if (feed.files.length === 0) {
    throw new Error("The previous update feed has no files.");
  }
  const absolute = (url) => {
    if (typeof url !== "string" || url === "") {
      throw new Error("An update feed file has no URL.");
    }
    const resolved = new URL(url, base);
    if (resolved.protocol !== "https:") {
      throw new Error(`An update feed file is not served over HTTPS: ${url}`);
    }
    return resolved.href;
  };
  const files = feed.files.map((file) => {
    if (!/^[A-Za-z0-9+/]{86}==$/.test(file.sha512 ?? "")) {
      throw new Error(`${file.url}: the SHA-512 checksum is missing.`);
    }
    return { ...file, url: absolute(file.url) };
  });
  return {
    ...feed,
    files,
    path: feed.path === undefined ? files[0].url : absolute(feed.path),
  };
}

if (require.main === module) {
  // node tools/carry-update-feed.cjs <previous feed> <previous release download folder>
  try {
    const [file, downloadBase] = process.argv.slice(2);
    if (!file || !downloadBase) {
      throw new Error("Usage: carry-update-feed.cjs <feed> <download folder>");
    }
    const feed = JSON.parse(fs.readFileSync(file, "utf8"));
    process.stdout.write(
      `${JSON.stringify(carry(feed, downloadBase), null, 2)}\n`,
    );
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

module.exports = { carry };
