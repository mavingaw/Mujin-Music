const test = require("node:test");
const assert = require("node:assert/strict");
const {
  commitsUrl,
  latestDownloadBase,
  releasesBase,
  sourceFileUrl,
  tagDownloadBase,
} = require("./releases-base.cjs");
const { carry } = require("./carry-update-feed.cjs");

test("defaults to the GitHub releases and derives each download folder", () => {
  const base = releasesBase({});
  assert.equal(base, "https://github.com/mavingaw/Mujin-Music/releases");
  assert.equal(latestDownloadBase(base), `${base}/latest/download`);
  assert.equal(tagDownloadBase(base, "v1.2.3"), `${base}/download/v1.2.3`);
  assert.equal(
    sourceFileUrl(base, "v1.2.3", "docs/server.md"),
    "https://github.com/mavingaw/Mujin-Music/blob/v1.2.3/docs/server.md",
  );
  assert.equal(
    commitsUrl(base, "v1.2.3"),
    "https://github.com/mavingaw/Mujin-Music/commits/v1.2.3",
  );
});

test("a Gitea releases page uses Gitea's latest and source paths", () => {
  const base = releasesBase({
    MUJIN_RELEASES_BASE:
      " https://git.example.test/owner/Mujin-Music/releases/ ",
  });
  assert.equal(base, "https://git.example.test/owner/Mujin-Music/releases");
  assert.equal(latestDownloadBase(base), `${base}/download/latest`);
  assert.equal(
    sourceFileUrl(base, "v1.2.3", "docs/server.md"),
    "https://git.example.test/owner/Mujin-Music/src/tag/v1.2.3/docs/server.md",
  );
  assert.equal(
    commitsUrl(base, "v1.2.3"),
    "https://git.example.test/owner/Mujin-Music/commits/tag/v1.2.3",
  );
});

test("refuses a releases page clients could not trust", () => {
  for (const value of [
    "http://192.168.1.58:3002/owner/repo/releases",
    "https://git.example.test/owner/repo",
    "https://git.example.test/releases",
    "https://git.example.test/owner/repo/releases?x=1",
    "https://user:pass@git.example.test/owner/repo/releases",
    "not a url",
  ]) {
    assert.throws(() => releasesBase({ MUJIN_RELEASES_BASE: value }), value);
  }
  assert.equal(
    releasesBase(
      { MUJIN_RELEASES_BASE: "http://192.168.1.58:3002/owner/repo/releases" },
      { allowHttp: true },
    ),
    "http://192.168.1.58:3002/owner/repo/releases",
  );
});

test("carries a feed over with absolute links to the previous release", () => {
  const sha512 = `${"A".repeat(86)}==`;
  const previous = {
    version: "1.1.0",
    files: [
      { url: "Mujin-Music-1.1.0-x64.zip", size: 1, sha512 },
      {
        url: "https://git.example.test/o/r/releases/download/v1.0.0/Mujin-Music-1.0.0-arm64.zip",
        size: 2,
        sha512,
      },
    ],
    path: "Mujin-Music-1.1.0-x64.zip",
    sha512,
    releaseDate: "2026-01-01T00:00:00.000Z",
  };
  const carried = carry(
    previous,
    "https://git.example.test/o/r/releases/download/v1.1.0",
  );
  assert.equal(carried.version, "1.1.0");
  assert.deepEqual(
    carried.files.map((file) => file.url),
    [
      "https://git.example.test/o/r/releases/download/v1.1.0/Mujin-Music-1.1.0-x64.zip",
      "https://git.example.test/o/r/releases/download/v1.0.0/Mujin-Music-1.0.0-arm64.zip",
    ],
  );
  assert.equal(carried.path, carried.files[0].url);
  assert.equal(carried.releaseDate, previous.releaseDate);

  assert.throws(() =>
    carry(previous, "http://192.168.1.58:3002/o/r/releases/download/v1.1.0"),
  );
  assert.throws(() => carry({ ...previous, files: [] }, "https://x.test/d"));
  assert.throws(() =>
    carry(
      { ...previous, files: [{ url: "a.zip", size: 1 }] },
      "https://x.test/d",
    ),
  );
  assert.throws(() =>
    carry(
      { ...previous, files: [{ url: "http://x.test/a.zip", size: 1, sha512 }] },
      "https://x.test/d",
    ),
  );
});
