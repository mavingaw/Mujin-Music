/**
 * Where Mujin Music releases are downloaded from.
 *
 * One build-time setting, MUJIN_RELEASES_BASE, names the releases page of
 * the repository that publishes the packages, for example
 *
 *   https://github.com/mavingaw/Mujin-Music/releases      (the default)
 *   https://git.example.com/mavingaw/Mujin-Music/releases (Gitea or Forgejo)
 *
 * Every download and update URL is derived from it:
 *
 *   tag download     <base>/download/<tag>/<file>        (all of them)
 *   latest download  <base>/latest/download/<file>       (GitHub)
 *                    <base>/download/latest/<file>       (Gitea and Forgejo)
 *   latest page      <base>/latest                       (both)
 *
 * Both "latest" forms resolve to the newest release that is neither a draft
 * nor a pre-release. crates/windows-server/src/updater.rs and
 * crates/windows-server/package.ps1 apply the same rules.
 */
const defaultReleasesBase = "https://github.com/mavingaw/Mujin-Music/releases";

/**
 * MUJIN_RELEASES_BASE, checked. Clients only ever update over HTTPS;
 * allowHttp is for links in release notes on a LAN-only forge.
 */
function releasesBase(environment = process.env, { allowHttp = false } = {}) {
  const configured = (environment.MUJIN_RELEASES_BASE ?? "").trim();
  const base = (configured || defaultReleasesBase).replace(/\/+$/, "");
  let url;
  try {
    url = new URL(base);
  } catch {
    throw new Error(`MUJIN_RELEASES_BASE is not a URL: ${base}`);
  }
  if (url.protocol !== "https:" && !(allowHttp && url.protocol === "http:")) {
    throw new Error("MUJIN_RELEASES_BASE must use HTTPS.");
  }
  if (url.search || url.hash || url.username || url.password) {
    throw new Error(
      "MUJIN_RELEASES_BASE must not carry a query, fragment or credentials.",
    );
  }
  const segments = url.pathname.split("/").filter(Boolean);
  if (segments.length < 3 || segments.at(-1) !== "releases") {
    throw new Error(
      "MUJIN_RELEASES_BASE must be a repository's releases page, such as https://git.example.com/owner/repo/releases.",
    );
  }
  return base;
}

function isGitHub(base) {
  return new URL(base).hostname === "github.com";
}

/** The folder of one tagged release's files. */
function tagDownloadBase(base, tag) {
  return `${base}/download/${tag}`;
}

/** The folder that always holds the newest stable release's files. */
function latestDownloadBase(base) {
  return isGitHub(base) ? `${base}/latest/download` : `${base}/download/latest`;
}

/** The repository's web page. */
function repositoryUrl(base) {
  return base.replace(/\/releases$/, "");
}

/** A file in the repository at a tag, as the forge's web UI shows it. */
function sourceFileUrl(base, tag, file) {
  const kind = isGitHub(base) ? "blob" : "src/tag";
  return `${repositoryUrl(base)}/${kind}/${tag}/${file}`;
}

/** The commit history up to a tag. */
function commitsUrl(base, tag) {
  return isGitHub(base)
    ? `${repositoryUrl(base)}/commits/${tag}`
    : `${repositoryUrl(base)}/commits/tag/${tag}`;
}

module.exports = {
  commitsUrl,
  defaultReleasesBase,
  isGitHub,
  latestDownloadBase,
  releasesBase,
  repositoryUrl,
  sourceFileUrl,
  tagDownloadBase,
};

if (require.main === module) {
  // Prints one derived URL, for shell scripts:
  //   node tools/releases-base.cjs [base|latest|tag <tag>|repository]
  const [what = "base", tag] = process.argv.slice(2);
  try {
    const base = releasesBase();
    const value = {
      base: () => base,
      latest: () => latestDownloadBase(base),
      tag: () => {
        if (!/^v\d+\.\d+\.\d+$/.test(tag ?? "")) {
          throw new Error("Pass a release tag such as v1.2.3.");
        }
        return tagDownloadBase(base, tag);
      },
      repository: () => repositoryUrl(base),
    }[what];
    if (!value) {
      throw new Error(`Unknown value ${what}.`);
    }
    process.stdout.write(`${value()}\n`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
