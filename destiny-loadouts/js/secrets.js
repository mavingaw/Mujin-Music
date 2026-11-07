// Keeps your Bungie API key, OAuth client secret and sign-in tokens
// encrypted at rest.
//
// - Desktop app: encrypted by Windows (DPAPI) through Electron's safeStorage,
//   in the app's own data folder. See desktop/main.cjs.
// - Browser: encrypted with AES-GCM using a key the browser generates and
//   keeps as non-extractable in IndexedDB, so the stored text is useless if
//   copied elsewhere.
//
// Values are loaded once at startup (loadSecrets) and then read from memory.

const desktop = globalThis.desktop;
const PREFIX = "d2lo.secret.";
const KEY_DB = "d2lo-keys";
const cache = new Map();

// ---------- browser: WebCrypto ----------

function openKeyDb() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(KEY_DB, 1);
    request.onupgradeneeded = () => request.result.createObjectStore("keys");
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function browserKey() {
  const db = await openKeyDb();
  const get = () =>
    new Promise((resolve, reject) => {
      const r = db.transaction("keys").objectStore("keys").get("main");
      r.onsuccess = () => resolve(r.result);
      r.onerror = () => reject(r.error);
    });
  let key = await get();
  if (key) return key;
  key = await crypto.subtle.generateKey(
    { name: "AES-GCM", length: 256 },
    false,
    ["encrypt", "decrypt"],
  );
  await new Promise((resolve, reject) => {
    const tx = db.transaction("keys", "readwrite");
    tx.objectStore("keys").put(key, "main");
    tx.oncomplete = resolve;
    tx.onerror = () => reject(tx.error);
  });
  return key;
}

const toBase64 = (bytes) => btoa(String.fromCharCode(...bytes));
const fromBase64 = (text) =>
  Uint8Array.from(atob(text), (c) => c.charCodeAt(0));

async function browserEncrypt(value) {
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const data = new TextEncoder().encode(JSON.stringify(value));
  const sealed = new Uint8Array(
    await crypto.subtle.encrypt(
      { name: "AES-GCM", iv },
      await browserKey(),
      data,
    ),
  );
  return `${toBase64(iv)}.${toBase64(sealed)}`;
}

async function browserDecrypt(text) {
  const [iv, sealed] = text.split(".").map(fromBase64);
  const data = await crypto.subtle.decrypt(
    { name: "AES-GCM", iv },
    await browserKey(),
    sealed,
  );
  return JSON.parse(new TextDecoder().decode(data));
}

// ---------- public API ----------

/** Loads the stored values for `names` into memory. Call once at startup. */
export async function loadSecrets(names) {
  if (desktop?.secrets) {
    const values = await desktop.secrets.load();
    for (const name of names) {
      if (values?.[name] !== undefined) cache.set(name, values[name]);
    }
    return;
  }
  for (const name of names) {
    try {
      const stored = localStorage.getItem(PREFIX + name);
      if (stored) cache.set(name, await browserDecrypt(stored));
    } catch {
      // Unreadable (a different browser profile, or cleared site data):
      // treat it as not stored.
      localStorage.removeItem(PREFIX + name);
    }
    // Earlier versions kept these unencrypted; drop those copies.
    localStorage.removeItem(name);
  }
}

export function getSecret(name) {
  return cache.get(name) ?? null;
}

/** Stores a value (or removes it when value is null). Resolves once saved. */
export async function setSecret(name, value) {
  if (value === null) cache.delete(name);
  else cache.set(name, value);
  if (desktop?.secrets) {
    await desktop.secrets.save(name, value);
    return;
  }
  try {
    if (value === null) localStorage.removeItem(PREFIX + name);
    else localStorage.setItem(PREFIX + name, await browserEncrypt(value));
  } catch {
    // Storage refused (private window): it still works until the page closes.
  }
}
