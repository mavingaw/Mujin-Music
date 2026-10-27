const fs = require("node:fs");
const path = require("node:path");

const workspace = path.resolve(__dirname, "..");
const output = path.join(workspace, "THIRD_PARTY_NOTICES.md");
const notice = `# Third-party notices

Mujin Music is based on Parson by ParsonLabs and, like Parson, is distributed
under the GNU General Public License version 3 only. The complete license text
is included as \`LICENSE\`.

Binary packages also contain third-party software. Those components retain
their own licenses:

- Desktop packages include Electron and Chromium license files.
- Rust dependencies and their exact versions are listed in \`Cargo.lock\`.
- JavaScript dependencies and their exact versions are listed in \`bun.lock\`.
- Container images retain operating-system package notices under
  \`/usr/share/doc\`.
- The web app bundles the Jost, Inter, JetBrains Mono, Fraunces, Atkinson
  Hyperlegible Next, IBM Plex Sans and Roboto Flex fonts under the SIL Open
  Font License 1.1; each font folder in \`apps/web/public/fonts\` carries its
  \`OFL.txt\`.

The corresponding source and locked dependency manifests are available from
the release tag at <https://github.com/mavingaw/Mujin-Music>. The upstream
Parson source is at <https://github.com/ParsonLabs/Parson>.
`;

function generateThirdPartyNotices(destination = output) {
  fs.writeFileSync(destination, notice);
  return destination;
}

if (require.main === module) {
  process.stdout.write(`${generateThirdPartyNotices()}\n`);
}

module.exports = { generateThirdPartyNotices, notice };
