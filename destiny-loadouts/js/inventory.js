// Turns a Bungie GetProfile response into a flat item list the rest of the
// app can work with. Pure: no network, no DOM.

// Armor 3.0 stats, in the order the optimizer uses. The hashes are the same
// ones the game used before Armor 3.0 renamed the stats.
export const ARMOR_STAT_HASHES = [
  2996146975, // Weapons (was Mobility)
  392767087, // Health (was Resilience)
  1943323491, // Class (was Recovery)
  1735777505, // Grenade (was Discipline)
  144602215, // Super (was Intellect)
  4244567218, // Melee (was Strength)
];
export const FALLBACK_STAT_NAMES = [
  "Weapons",
  "Health",
  "Class",
  "Grenade",
  "Super",
  "Melee",
];

export const BUCKETS = {
  kinetic: 1498876634,
  energy: 2465295065,
  power: 953998645,
  helmet: 3448274439,
  gauntlets: 3551918588,
  chest: 14239492,
  legs: 20886954,
  classItem: 1585787867,
  subclass: 3284755031,
};
export const SLOT_BY_BUCKET = Object.fromEntries(
  Object.entries(BUCKETS).map(([slot, hash]) => [hash, slot]),
);
export const ARMOR_SLOTS = [
  "helmet",
  "gauntlets",
  "chest",
  "legs",
  "classItem",
];
export const WEAPON_SLOTS = ["kinetic", "energy", "power"];
export const POSTMASTER_BUCKET = 215593132;

export const CLASS_NAMES = { 0: "Titan", 1: "Hunter", 2: "Warlock" };

const LOCKED_STATE = 1;

/** Every item a character can hold besides the equipped one, per bucket. */
export const BUCKET_CAPACITY = 9;

function itemFrom(raw, owner, equipped, profile, manifest) {
  if (!raw.itemInstanceId) return null;
  if (raw.bucketHash === POSTMASTER_BUCKET) return null;
  const def = manifest.items[raw.itemHash];
  if (!def) return null;
  const slot = SLOT_BY_BUCKET[def.bucket];
  if (!slot) return null;

  const id = raw.itemInstanceId;
  const instance = profile.itemComponents?.instances?.data?.[id] ?? {};
  const statValues = profile.itemComponents?.stats?.data?.[id]?.stats ?? {};
  const isArmor = ARMOR_SLOTS.includes(slot);

  return {
    id,
    hash: raw.itemHash,
    name: def.name,
    icon: def.icon,
    watermark: def.watermark,
    typeName: def.typeName,
    slot,
    bucket: def.bucket,
    classType: def.classType,
    exotic: def.exotic,
    isArmor,
    isWeapon: WEAPON_SLOTS.includes(slot),
    owner,
    equipped,
    locked: (raw.state & LOCKED_STATE) !== 0,
    power: instance.primaryStat?.value ?? instance.itemLevel ?? 0,
    canEquip: instance.canEquip !== false,
    stats: isArmor
      ? ARMOR_STAT_HASHES.map((hash) => statValues[hash]?.value ?? 0)
      : null,
  };
}

/**
 * Returns { characters, items, gameLoadouts }.
 *   characters: [{ id, classType, className, light, emblem, emblemBackground }]
 *   items: every weapon, armor piece and subclass, wherever it is
 *   gameLoadouts: { [characterId]: the in-game loadouts }
 */
export function buildInventory(profile, manifest) {
  const characterData = profile.characters?.data ?? {};
  const characters = Object.values(characterData)
    .sort((a, b) => new Date(b.dateLastPlayed) - new Date(a.dateLastPlayed))
    .map((c) => ({
      id: c.characterId,
      classType: c.classType,
      className: CLASS_NAMES[c.classType] ?? "Guardian",
      light: c.light,
      emblem: c.emblemPath,
      emblemBackground: c.emblemBackgroundPath,
    }));

  const items = [];
  const add = (raw, owner, equipped) => {
    const item = itemFrom(raw, owner, equipped, profile, manifest);
    if (item) items.push(item);
  };

  for (const raw of profile.profileInventory?.data?.items ?? []) {
    add(raw, "vault", false);
  }
  for (const [charId, inv] of Object.entries(
    profile.characterInventories?.data ?? {},
  )) {
    for (const raw of inv.items) add(raw, charId, false);
  }
  for (const [charId, inv] of Object.entries(
    profile.characterEquipment?.data ?? {},
  )) {
    for (const raw of inv.items) add(raw, charId, true);
  }

  const gameLoadouts = {};
  for (const [charId, data] of Object.entries(
    profile.characterLoadouts?.data ?? {},
  )) {
    gameLoadouts[charId] = data.loadouts ?? [];
  }

  return { characters, items, gameLoadouts };
}

/** The armor a character could wear, grouped by slot for the optimizer. */
export function armorForClass(items, classType) {
  const bySlot = Object.fromEntries(ARMOR_SLOTS.map((s) => [s, []]));
  for (const item of items) {
    if (!item.isArmor || item.classType !== classType) continue;
    bySlot[item.slot].push({
      id: item.id,
      hash: item.hash,
      slot: item.slot,
      exotic: item.exotic,
      stats: item.stats,
    });
  }
  return bySlot;
}

/** Sums the armor stats of what a character has equipped. */
export function equippedArmorTotals(items, characterId) {
  const totals = [0, 0, 0, 0, 0, 0];
  for (const item of items) {
    if (item.owner !== characterId || !item.equipped || !item.isArmor) continue;
    item.stats.forEach((v, i) => (totals[i] += v));
  }
  return totals;
}

export function statNames(manifest) {
  return ARMOR_STAT_HASHES.map(
    (hash, i) => manifest?.stats?.[hash]?.name || FALLBACK_STAT_NAMES[i],
  );
}
