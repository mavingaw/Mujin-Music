// Serves the app at https://localhost:55555/.
//
// Bungie only redirects back to HTTPS addresses, so this makes a self-signed
// certificate for localhost the first time it runs (it needs `openssl`, which
// ships with macOS, Linux and Git for Windows). Your browser will warn about
// the certificate once; choose "Advanced" then "Proceed to localhost".

import { createServer } from "node:https";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { extname, join, normalize, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL(".", import.meta.url));
const PORT = Number(process.env.PORT ?? 55555);
const CERT_DIR = join(ROOT, ".certs");
const KEY = join(CERT_DIR, "localhost-key.pem");
const CERT = join(CERT_DIR, "localhost-cert.pem");

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
};

function ensureCertificate() {
  if (existsSync(KEY) && existsSync(CERT)) return;
  mkdirSync(CERT_DIR, { recursive: true });
  console.log("Creating a self-signed certificate for localhost…");
  try {
    execFileSync(
      "openssl",
      [
        "req",
        "-x509",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-sha256",
        "-days",
        "825",
        "-subj",
        "/CN=localhost",
        "-addext",
        "subjectAltName=DNS:localhost,IP:127.0.0.1",
        "-keyout",
        KEY,
        "-out",
        CERT,
      ],
      { stdio: "ignore" },
    );
  } catch {
    console.error(
      "Could not run openssl to create a certificate.\n" +
        "Install OpenSSL (or Git for Windows, which includes it) and run this again,\n" +
        `or put your own key and certificate at:\n  ${KEY}\n  ${CERT}`,
    );
    process.exit(1);
  }
}

ensureCertificate();

const server = createServer(
  { key: readFileSync(KEY), cert: readFileSync(CERT) },
  async (req, res) => {
    const url = new URL(req.url, `https://${req.headers.host}`);
    let path = decodeURIComponent(url.pathname);
    if (path.endsWith("/")) path += "index.html";
    const file = resolve(ROOT, normalize(`.${path}`));
    const hidden = file
      .slice(ROOT.length)
      .split(sep)
      .some((part) => part.startsWith(".") || part === "node_modules");
    if (!file.startsWith(ROOT) || hidden) {
      res.writeHead(404).end("Not found");
      return;
    }
    try {
      const body = await readFile(file);
      res.writeHead(200, {
        "Content-Type": TYPES[extname(file)] ?? "application/octet-stream",
        "Cache-Control": "no-cache",
      });
      res.end(body);
    } catch {
      res.writeHead(404).end("Not found");
    }
  },
);

server.listen(PORT, () => {
  console.log(`Guardian Loadouts is running at https://localhost:${PORT}/`);
});
