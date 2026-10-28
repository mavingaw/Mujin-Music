const fs = require("node:fs");
const path = require("node:path");
const { versionParts } = require("./release.cjs");

const templatePath = path.resolve(
  __dirname,
  "..",
  ".github",
  "release-notes.md",
);
const signingKeys = ["upload", "debug"];
const signingBlock =
  /<!-- android-signing: ([a-z]+) -->\n([\s\S]*?)<!-- end android-signing -->\n?/g;

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
function render(template, { version, repository, signingKey, certificate }) {
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
    image: `ghcr.io/${repository.toLowerCase()}`,
    certificate,
  };
  const body = stripComments(
    template.replace(signingBlock, (_, key, text) =>
      key === signingKey ? text : "",
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

function main(args) {
  const [version, repository, signingKey, certificate] = args;
  const template = fs.readFileSync(templatePath, "utf8");
  process.stdout.write(
    render(template, { version, repository, signingKey, certificate }),
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
