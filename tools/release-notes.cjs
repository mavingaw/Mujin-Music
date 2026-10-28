const fs = require("node:fs");
const path = require("node:path");
const { versionParts } = require("./release.cjs");
const {
  commitsUrl,
  releasesBase: configuredReleasesBase,
  sourceFileUrl,
  tagDownloadBase,
} = require("./releases-base.cjs");

const templatePath = path.resolve(
  __dirname,
  "..",
  ".github",
  "release-notes.md",
);
const signingKeys = ["upload", "debug"];
const signingBlock =
  /<!-- android-signing: ([a-z]+) -->\n([\s\S]*?)<!-- end android-signing -->\n?/g;
const provenanceKinds = ["attested", "checksums"];
const provenanceBlock =
  /<!-- provenance: ([a-z]+) -->\n([\s\S]*?)<!-- end provenance -->\n?/g;
// Optional parts may sit inside a table cell, so they need no line breaks.
const optionalBlock =
  /<!-- optional: ([a-z0-9-]+) -->([\s\S]*?)<!-- end optional -->/g;
const optionalParts = ["linux-arm64"];

/**
 * Removes HTML comments, repeating until none are left so that a comment
 * nested inside another cannot leave a new "<!--" behind, and rejects a
 * comment that is never closed.
 */
function stripComments(text) {
  let previous;
  do {
    previous = text;
    text = text.replace(/<!--[\s\S]*?-->\n*/g, "");
  } while (text !== previous);
  if (text.includes("<!--")) {
    throw new Error("The release notes template has an unclosed comment.");
  }
  return text;
}

/**
 * Renders .github/release-notes.md for one release: keeps the Android
 * signing block for the key that signed the APKs, removes the template's
 * comments and fills in the placeholders.
 */
function render(
  template,
  {
    version,
    repository,
    signingKey,
    certificate,
    releasesBase,
    image,
    provenance = "attested",
    without = [],
  },
) {
  versionParts(version);
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository ?? "")) {
    throw new Error(`Not a GitHub repository name: ${repository}`);
  }
  if (!signingKeys.includes(signingKey)) {
    throw new Error(
      `The Android signing key must be one of ${signingKeys.join(", ")}.`,
    );
  }
  if (!/^[0-9a-f]{64}$/.test(certificate ?? "")) {
    throw new Error("The Android certificate must be a SHA-256 digest in hex.");
  }

  if (!provenanceKinds.includes(provenance)) {
    throw new Error(
      `The provenance must be one of ${provenanceKinds.join(", ")}.`,
    );
  }
  for (const part of without) {
    if (!optionalParts.includes(part)) {
      throw new Error(`Unknown optional part ${part}.`);
    }
  }
  // Without an explicit releases page, the GitHub repository's.
  // A LAN-only Gitea serves its own release page over plain HTTP.
  const base = configuredReleasesBase(
    {
      MUJIN_RELEASES_BASE:
        releasesBase || `https://github.com/${repository}/releases`,
    },
    { allowHttp: true },
  );
  const tag = `v${version}`;
  if (image !== undefined && !/^[a-z0-9.:-]+(\/[a-z0-9._-]+)+$/.test(image)) {
    throw new Error(`Not a container image name: ${image}`);
  }

  const blocks = [...template.matchAll(signingBlock)].map((match) => match[1]);
  for (const key of signingKeys) {
    if (blocks.filter((block) => block === key).length !== 1) {
      throw new Error(
        `The template needs exactly one android-signing block for "${key}".`,
      );
    }
  }

  const values = {
    version,
    repository,
    downloads: tagDownloadBase(base, tag),
    serverGuide: sourceFileUrl(base, tag, "docs/server.md"),
    commits: commitsUrl(base, tag),
    image: image ?? `ghcr.io/${repository.toLowerCase()}`,
    certificate,
  };
  const body = stripComments(
    template
      .replace(signingBlock, (_, key, text) => (key === signingKey ? text : ""))
      .replace(provenanceBlock, (_, kind, text) =>
        kind === provenance ? text : "",
      )
      .replace(optionalBlock, (_, part, text) =>
        without.includes(part) ? "" : text,
      ),
  )
    .replace(/\{\{(\w+)\}\}/g, (placeholder, name) => {
      if (!Object.hasOwn(values, name)) {
        throw new Error(`Unknown placeholder ${placeholder}.`);
      }
      return values[name];
    })
    .replace(/\n{3,}/g, "\n\n")
    .trim();
  return `${body}\n`;
}

/**
 * node tools/release-notes.cjs <version> <owner/name> <signing key> <certificate>
 *
 * The environment chooses the rest: MUJIN_RELEASES_BASE (where the files are
 * downloaded from), MUJIN_CONTAINER_IMAGE (the image to pull),
 * MUJIN_PROVENANCE ("attested" or "checksums") and MUJIN_WITHOUT (a comma
 * separated list of optional parts the release does not have, such as
 * linux-arm64).
 */
function main(args, environment = process.env) {
  const [version, repository, signingKey, certificate] = args;
  const template = fs.readFileSync(templatePath, "utf8");
  process.stdout.write(
    render(template, {
      version,
      repository,
      signingKey,
      certificate,
      releasesBase: environment.MUJIN_RELEASES_BASE || undefined,
      image: environment.MUJIN_CONTAINER_IMAGE || undefined,
      provenance: environment.MUJIN_PROVENANCE || undefined,
      without: (environment.MUJIN_WITHOUT || "").split(",").filter(Boolean),
    }),
  );
}

if (require.main === module) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

module.exports = { render, stripComments, templatePath };
