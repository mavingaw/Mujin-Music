// Sign-in per Parson server identity, not per host.
//
// Browsers scope cookies by host name and ignore the port, so two Parson
// servers on one machine (127.0.0.1:1993 and 127.0.0.1:2993) would share a
// sign-in, and one server reached first at 127.0.0.1 and then at its LAN
// address would ask for a second sign-in. Parson Desktop shows one server at
// a time, so it keeps each server's cookies in a vault keyed by the server's
// instanceId and moves them to whichever address that server is reached at.
//
// `store` is the cookie jar: get(origin) -> cookies, set(origin, cookie),
// remove(origin, cookie). Electron's session.cookies is adapted in main.cjs;
// tests use an in-memory jar.
const nodeFs = require("node:fs");
const path = require("node:path");

const VAULT_VERSION = 1;
const SIGN_IN_COOKIE = "plm_refreshToken";
const MAX_SERVERS = 40;

function emptyVault() {
  return { version: VAULT_VERSION, servers: {}, hosts: {} };
}

function hostOf(origin) {
  try {
    return new URL(origin).hostname;
  } catch {
    return "";
  }
}

function sanitizeCookie(cookie) {
  if (!cookie || typeof cookie.name !== "string") return null;
  return {
    name: cookie.name,
    value: String(cookie.value ?? ""),
    path: typeof cookie.path === "string" && cookie.path ? cookie.path : "/",
    secure: cookie.secure === true,
    httpOnly: cookie.httpOnly === true,
    sameSite:
      typeof cookie.sameSite === "string" ? cookie.sameSite : "unspecified",
    ...(Number.isFinite(cookie.expirationDate)
      ? { expirationDate: cookie.expirationDate }
      : {}),
  };
}

function parseVault(value) {
  const vault = emptyVault();
  if (!value || typeof value !== "object") return vault;
  for (const [id, entry] of Object.entries(value.servers ?? {})) {
    if (!id || !Array.isArray(entry?.cookies)) continue;
    vault.servers[id] = {
      cookies: entry.cookies.map(sanitizeCookie).filter(Boolean),
      savedAt: Number.isFinite(entry.savedAt) ? entry.savedAt : 0,
    };
  }
  for (const [host, id] of Object.entries(value.hosts ?? {})) {
    if (host && typeof id === "string" && id) vault.hosts[host] = id;
  }
  return vault;
}

function expired(cookie, nowSeconds) {
  return (
    Number.isFinite(cookie.expirationDate) &&
    cookie.expirationDate <= nowSeconds
  );
}

function hasSignIn(cookies, now = Date.now()) {
  const seconds = now / 1000;
  return cookies.some(
    (cookie) =>
      cookie.name === SIGN_IN_COOKIE &&
      cookie.value &&
      !expired(cookie, seconds),
  );
}

function createSessionVault({ store, load, save, now = Date.now }) {
  let vault = parseVault(load?.());
  let queue = Promise.resolve();
  // Every change runs in order: cookie events can arrive mid-switch.
  const serial = (task) => {
    const next = queue.then(task, task);
    queue = next.catch(() => {});
    return next;
  };

  function persist() {
    const ids = Object.entries(vault.servers)
      .sort((left, right) => right[1].savedAt - left[1].savedAt)
      .slice(0, MAX_SERVERS)
      .map(([id]) => id);
    vault.servers = Object.fromEntries(
      ids.map((id) => [id, vault.servers[id]]),
    );
    save?.(vault);
  }

  async function snapshot(id, host) {
    const cookies = (await store.get(`http://${host}`))
      .map(sanitizeCookie)
      .filter(Boolean);
    vault.servers[id] = { cookies, savedAt: now() };
    return cookies;
  }

  async function clearHost(host, cookies) {
    for (const cookie of cookies) {
      await store.remove(`http://${host}`, cookie);
    }
  }

  return {
    /** The vault as stored (for tests and diagnostics). */
    get state() {
      return vault;
    },

    /**
     * Makes the jar hold server `id`'s sign-in for `origin`, and nobody
     * else's. Call before loading that server's web app.
     */
    activate(id, origin) {
      return serial(async () => {
        const host = hostOf(origin);
        if (!id || !host) return;
        // The same server, reached before at another address: move its
        // cookies here so they exist (and rotate) in one place only.
        for (const [otherHost, owner] of Object.entries(vault.hosts)) {
          if (owner !== id || otherHost === host) continue;
          const cookies = await snapshot(id, otherHost);
          await clearHost(otherHost, cookies);
          delete vault.hosts[otherHost];
        }
        const owner = vault.hosts[host];
        if (owner !== id) {
          if (owner) {
            // Another server's sign-in lives on this host (another port).
            const cookies = await snapshot(owner, host);
            await clearHost(host, cookies);
          }
          // With no recorded owner (an older Parson Desktop), whatever is
          // there is adopted as this server's.
          const saved = vault.servers[id]?.cookies ?? [];
          if (saved.length) {
            const current = await store.get(`http://${host}`);
            await clearHost(host, current);
            const seconds = now() / 1000;
            const https = origin.startsWith("https:");
            for (const cookie of saved) {
              if (expired(cookie, seconds) || (cookie.secure && !https)) {
                continue;
              }
              await store.set(origin, cookie);
            }
          }
          vault.hosts[host] = id;
        }
        await snapshot(id, host);
        persist();
      });
    },

    /** Saves the active server's cookies after they change. */
    capture(origin) {
      return serial(async () => {
        const host = hostOf(origin);
        const id = vault.hosts[host];
        if (!id) return;
        await snapshot(id, host);
        persist();
      });
    },

    /** Signs this computer out of server `id` everywhere. */
    forget(id) {
      return serial(async () => {
        for (const [host, owner] of Object.entries(vault.hosts)) {
          if (owner !== id) continue;
          await clearHost(host, await store.get(`http://${host}`));
          delete vault.hosts[host];
        }
        delete vault.servers[id];
        persist();
      });
    },

    /** Instance ids this computer holds a live sign-in for. */
    signedIn() {
      return serial(async () => {
        const ids = new Set();
        for (const [id, entry] of Object.entries(vault.servers)) {
          if (hasSignIn(entry.cookies, now())) ids.add(id);
        }
        // The jar is the truth for servers whose cookies live there now.
        for (const [host, id] of Object.entries(vault.hosts)) {
          const live = (await store.get(`http://${host}`))
            .map(sanitizeCookie)
            .filter(Boolean);
          if (hasSignIn(live, now())) ids.add(id);
          else ids.delete(id);
        }
        return ids;
      });
    },
  };
}

/** Keeps the vault in a file, encrypted with the OS keychain if possible. */
function fileVaultStorage({ file, safeStorage = null, fs = nodeFs }) {
  const encrypt =
    safeStorage?.isEncryptionAvailable?.() === true ? safeStorage : null;
  return {
    load() {
      try {
        const raw = fs.readFileSync(file);
        const text =
          raw[0] === 0x7b
            ? raw.toString("utf8")
            : encrypt
              ? encrypt.decryptString(raw)
              : "";
        return text ? JSON.parse(text) : null;
      } catch {
        return null;
      }
    },
    save(vault) {
      try {
        fs.mkdirSync(path.dirname(file), { recursive: true });
        const text = JSON.stringify(vault);
        const temporary = `${file}.${process.pid}.tmp`;
        fs.writeFileSync(
          temporary,
          encrypt ? encrypt.encryptString(text) : text,
          { mode: 0o600 },
        );
        fs.renameSync(temporary, file);
      } catch (error) {
        console.error("Could not save server sign-ins", error);
      }
    },
  };
}

/** Adapts Electron's session.cookies to the jar the vault expects. */
function electronCookieStore(cookies) {
  return {
    async get(url) {
      const host = new URL(url).hostname;
      // Every path on this exact host (not its subdomains).
      return (await cookies.get({ domain: host })).filter(
        (cookie) => cookie.domain === host || cookie.domain === `.${host}`,
      );
    },
    async set(origin, cookie) {
      await cookies.set({
        url: `${origin}${cookie.path || "/"}`,
        name: cookie.name,
        value: cookie.value,
        path: cookie.path,
        secure: cookie.secure,
        httpOnly: cookie.httpOnly,
        sameSite: cookie.sameSite,
        ...(Number.isFinite(cookie.expirationDate)
          ? { expirationDate: cookie.expirationDate }
          : {}),
      });
    },
    async remove(url, cookie) {
      await cookies.remove(`${url}${cookie.path || "/"}`, cookie.name);
    },
  };
}

module.exports = {
  SIGN_IN_COOKIE,
  createSessionVault,
  electronCookieStore,
  emptyVault,
  fileVaultStorage,
  hasSignIn,
  parseVault,
};
