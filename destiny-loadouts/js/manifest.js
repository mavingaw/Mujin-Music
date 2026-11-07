// Downloads the parts of the Destiny 2 manifest this app needs, keeps a
// trimmed copy in IndexedDB and only downloads again after a game update.

import { api, fetchContent } from "./bungie.js";

const DB_NAME = "d2lo-manifest";
const STORE = "tables";
const CACHE_KEY = "slim-v2";

// itemType values worth keeping: armor, weapons and subclasses.
const ITEM_TYPES = new Set([2, 3, 16]);

function openDb() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, 1);
    request.onupgradeneeded = () => request.result.createObjectStore(STORE);
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function cacheGet(key) {
  try {
    const db = await openDb();
    return await new Promise((resolve, reject) => {
      const request = db.transaction(STORE).objectStore(STORE).get(key);
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
  } catch {
    return undefined;
  }
}

async function cachePut(key, value) {
  try {
    const db = await openDb();
    await new Promise((resolve, reject) => {
      const tx = db.transaction(STORE, "readwrite");
      tx.objectStore(STORE).put(value, key);
      tx.oncomplete = resolve;
      tx.onerror = () => reject(tx.error);
    });
  } catch {
    // A private window can refuse storage; the app still works, it just
    // downloads the manifest again next time.
  }
}

function slimItems(table) {
  const items = {};
  for (const [hash, def] of Object.entries(table)) {
    if (!ITEM_TYPES.has(def.itemType) || def.equippable === false) continue;
    if (def.redacted || !def.displayProperties?.name) continue;
    items[hash] = {
      name: def.displayProperties.name,
      icon: def.displayProperties.icon ?? "",
      type: def.itemType,
      typeName: def.itemTypeDisplayName ?? "",
      classType: def.classType,
      bucket: def.inventory?.bucketTypeHash ?? 0,
      exotic: def.inventory?.tierType === 6,
      watermark: def.iconWatermark ?? "",
    };
  }
  return items;
}

function slimStats(table) {
  const stats = {};
  for (const [hash, def] of Object.entries(table)) {
    stats[hash] = { name: def.displayProperties?.name ?? "" };
  }
  return stats;
}

function slimNamed(table) {
  const out = {};
  for (const [hash, def] of Object.entries(table ?? {})) {
    out[hash] = {
      name: def.name ?? def.displayProperties?.name ?? "",
      icon: def.iconImagePath ?? def.colorImagePath ?? "",
    };
  }
  return out;
}

/**
 * Armor sets with their 2- and 4-piece bonuses, plus which set each item
 * belongs to.
 */
function slimSets(setTable, perkTable) {
  const sets = {};
  const itemSet = {};
  for (const [hash, def] of Object.entries(setTable ?? {})) {
    if (def.redacted) continue;
    sets[hash] = {
      name: def.displayProperties?.name ?? "",
      perks: (def.setPerks ?? [])
        .map((p) => {
          const perk = perkTable?.[p.sandboxPerkHash]?.displayProperties ?? {};
          return {
            count: p.requiredSetCount,
            name: perk.name ?? "",
            description: perk.description ?? "",
          };
        })
        .sort((a, b) => a.count - b.count),
    };
    for (const item of def.setItems ?? []) itemSet[item] = Number(hash);
  }
  return { sets, itemSet };
}

/**
 * Returns { version, items, sets, itemSet, stats, loadoutNames, loadoutColors, loadoutIcons }.
 * onProgress receives short status strings for the loading screen.
 */
export async function loadManifest(onProgress = () => {}) {
  onProgress("Checking for game updates…");
  const manifest = await api("/Destiny2/Manifest/", { auth: false });
  const version = manifest.version;
  const cached = await cacheGet(CACHE_KEY);
  if (cached?.version === version) return cached;

  const paths = manifest.jsonWorldComponentContentPaths.en;
  const itemPath =
    paths.DestinyInventoryItemLiteDefinition ??
    paths.DestinyInventoryItemDefinition;

  onProgress("Downloading item definitions (one time per game update)…");
  const optional = (path) => (path ? fetchContent(path) : {});
  const [items, stats, names, colors, icons, setTable, perkTable] =
    await Promise.all([
      fetchContent(itemPath),
      fetchContent(paths.DestinyStatDefinition),
      paths.DestinyLoadoutNameDefinition
        ? fetchContent(paths.DestinyLoadoutNameDefinition)
        : {},
      paths.DestinyLoadoutColorDefinition
        ? fetchContent(paths.DestinyLoadoutColorDefinition)
        : {},
      paths.DestinyLoadoutIconDefinition
        ? fetchContent(paths.DestinyLoadoutIconDefinition)
        : {},
      optional(paths.DestinyEquipableItemSetDefinition),
      optional(paths.DestinySandboxPerkDefinition),
    ]);

  onProgress("Preparing item definitions…");
  const slim = {
    version,
    items: slimItems(items),
    stats: slimStats(stats),
    loadoutNames: slimNamed(names),
    loadoutColors: slimNamed(colors),
    loadoutIcons: slimNamed(icons),
    ...slimSets(setTable, perkTable),
  };
  await cachePut(CACHE_KEY, slim);
  return slim;
}
