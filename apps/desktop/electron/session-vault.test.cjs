const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const {
  SIGN_IN_COOKIE,
  createSessionVault,
  fileVaultStorage,
} = require("./session-vault.cjs");

/** A cookie jar like a browser's: scoped by host name, ignoring the port. */
function fakeJar() {
  const cookies = new Map();
  const key = (host, cookie) => `${host}|${cookie.path || "/"}|${cookie.name}`;
  return {
    cookies,
    async get(url) {
      const host = new URL(url).hostname;
      return [...cookies.entries()]
        .filter(([entry]) => entry.startsWith(`${host}|`))
        .map(([, cookie]) => ({ ...cookie }));
    },
    async set(origin, cookie) {
      cookies.set(key(new URL(origin).hostname, cookie), { ...cookie });
    },
    async remove(url, cookie) {
      cookies.delete(key(new URL(url).hostname, cookie));
    },
    // The web app signing in at an origin.
    signIn(origin, value) {
      cookies.set(key(new URL(origin).hostname, { name: SIGN_IN_COOKIE }), {
        name: SIGN_IN_COOKIE,
        value,
        path: "/",
        httpOnly: true,
      });
    },
    tokenAt(host) {
      return cookies.get(`${host}|/|${SIGN_IN_COOKIE}`)?.value ?? null;
    },
  };
}

const A = "instance-a";
const B = "instance-b";
const LOOPBACK_A = "http://127.0.0.1:1993";
const LOOPBACK_B = "http://127.0.0.1:2993";
const LAN_A = "http://192.168.1.20:1993";

test("two servers on one host no longer share a sign-in", async () => {
  const jar = fakeJar();
  const vault = createSessionVault({ store: jar });
  await vault.activate(A, LOOPBACK_A);
  jar.signIn(LOOPBACK_A, "token-a");
  await vault.capture(LOOPBACK_A);
  assert.deepEqual([...(await vault.signedIn())], [A]);

  // Server B on another port of the same host: A's cookie must not show B
  // as signed in, nor be sent to it.
  await vault.activate(B, LOOPBACK_B);
  assert.equal(jar.tokenAt("127.0.0.1"), null);
  assert.deepEqual([...(await vault.signedIn())], [A]);

  jar.signIn(LOOPBACK_B, "token-b");
  await vault.capture(LOOPBACK_B);
  assert.deepEqual([...(await vault.signedIn())].sort(), [A, B]);

  // Back to A: A's own sign-in returns, B's is kept for later.
  await vault.activate(A, LOOPBACK_A);
  assert.equal(jar.tokenAt("127.0.0.1"), "token-a");
  await vault.activate(B, LOOPBACK_B);
  assert.equal(jar.tokenAt("127.0.0.1"), "token-b");
});

test("one server reached at loopback and then the LAN signs in once", async () => {
  const jar = fakeJar();
  const vault = createSessionVault({ store: jar });
  await vault.activate(A, LOOPBACK_A);
  jar.signIn(LOOPBACK_A, "token-a");
  await vault.capture(LOOPBACK_A);

  await vault.activate(A, LAN_A);
  assert.equal(jar.tokenAt("192.168.1.20"), "token-a");
  // Moved, not copied: a rotated refresh token lives in one place only.
  assert.equal(jar.tokenAt("127.0.0.1"), null);
  assert.deepEqual([...(await vault.signedIn())], [A]);

  jar.signIn(LAN_A, "token-a-rotated");
  await vault.capture(LAN_A);
  await vault.activate(A, LOOPBACK_A);
  assert.equal(jar.tokenAt("127.0.0.1"), "token-a-rotated");
});

test("an older install's cookies are adopted by the first server", async () => {
  const jar = fakeJar();
  jar.signIn(LOOPBACK_A, "legacy");
  const vault = createSessionVault({ store: jar });
  await vault.activate(A, LOOPBACK_A);
  assert.equal(jar.tokenAt("127.0.0.1"), "legacy");
  assert.deepEqual([...(await vault.signedIn())], [A]);
});

test("expired or signed-out sessions are not shown as signed in", async () => {
  let now = 1_000_000;
  const jar = fakeJar();
  const vault = createSessionVault({ store: jar, now: () => now });
  await vault.activate(A, LOOPBACK_A);
  await jar.set(LOOPBACK_A, {
    name: SIGN_IN_COOKIE,
    value: "short",
    path: "/",
    expirationDate: now / 1000 + 60,
  });
  await vault.capture(LOOPBACK_A);
  await vault.activate(B, LOOPBACK_B);
  assert.deepEqual([...(await vault.signedIn())], [A]);
  now += 120_000;
  assert.deepEqual([...(await vault.signedIn())], []);

  // Signing out in the web app clears the cookie: the badge goes too.
  await vault.activate(A, LOOPBACK_A);
  await jar.remove(LOOPBACK_A, { name: SIGN_IN_COOKIE, path: "/" });
  await vault.capture(LOOPBACK_A);
  assert.deepEqual([...(await vault.signedIn())], []);
});

test("forgetting a server signs out of it everywhere", async () => {
  const jar = fakeJar();
  const vault = createSessionVault({ store: jar });
  await vault.activate(A, LAN_A);
  jar.signIn(LAN_A, "token-a");
  await vault.capture(LAN_A);
  await vault.activate(B, LOOPBACK_B);
  await vault.forget(A);
  assert.equal(jar.tokenAt("192.168.1.20"), null);
  assert.deepEqual([...(await vault.signedIn())], []);
  assert.equal(vault.state.servers[A], undefined);
});

test("the vault survives a restart and is encrypted when the OS allows", (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "parson-vault-"));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const file = path.join(directory, "server-sessions.bin");
  const safeStorage = {
    isEncryptionAvailable: () => true,
    encryptString: (text) =>
      Buffer.from(`enc:${Buffer.from(text).toString("base64")}`),
    decryptString: (buffer) =>
      Buffer.from(buffer.toString().slice(4), "base64").toString(),
  };
  const storage = fileVaultStorage({ file, safeStorage });
  storage.save({
    version: 1,
    servers: {
      [A]: { cookies: [{ name: SIGN_IN_COOKIE, value: "x" }], savedAt: 1 },
    },
    hosts: { "127.0.0.1": A },
  });
  assert.ok(!fs.readFileSync(file, "utf8").includes(SIGN_IN_COOKIE));
  assert.equal(fs.statSync(file).mode & 0o777, 0o600);
  const loaded = storage.load();
  assert.equal(loaded.hosts["127.0.0.1"], A);
  const vault = createSessionVault({ store: fakeJar(), load: () => loaded });
  assert.equal(vault.state.servers[A].cookies[0].value, "x");
});
