// Max Power: the highest-Power item for every slot. Pure, no DOM.
//
// A Guardian's Power is the average Power of all eight equipped items: three
// weapons and five armor pieces. The game allows one exotic weapon and one
// exotic armor piece at a time, so the best set is not always simply the
// top item in each slot.

import { ARMOR_SLOTS, WEAPON_SLOTS } from "./inventory.js";

export const POWER_SLOTS = [...WEAPON_SLOTS, ...ARMOR_SLOTS];

function canUse(item, classType) {
  return item.classType === classType || item.classType === 3;
}

/**
 * Best items for one group of slots (weapons or armor) with at most one
 * exotic among them. Returns { [slot]: item }.
 */
function bestGroup(items, slots) {
  const top = (slot, allowExotic) =>
    items
      .filter((i) => i.slot === slot && (allowExotic || !i.exotic))
      .sort((a, b) => b.power - a.power)[0];

  // Option 0: no exotic. Options 1..n: the exotic sits in slot n.
  let best = null;
  let bestSum = -Infinity;
  for (const exoticSlot of [null, ...slots]) {
    const pick = {};
    let sum = 0;
    for (const slot of slots) {
      const item =
        slot === exoticSlot
          ? items
              .filter((i) => i.slot === slot && i.exotic)
              .sort((a, b) => b.power - a.power)[0]
          : top(slot, false);
      if (!item) {
        // No exotic in that slot (or no legendary at all): try the next option.
        if (slot === exoticSlot) {
          sum = -Infinity;
          break;
        }
        continue;
      }
      pick[slot] = item;
      sum += item.power;
    }
    if (sum > bestSum) {
      bestSum = sum;
      best = pick;
    }
  }
  return best ?? {};
}

/**
 * Returns { items: { [slot]: item }, power, exactPower, missing } where power
 * is what the game shows (the average rounded down) and missing lists slots
 * with nothing usable.
 */
export function maxPowerLoadout(allItems, classType) {
  const usable = allItems.filter((i) => canUse(i, classType) && i.canEquip);
  const items = {
    ...bestGroup(
      usable.filter((i) => i.isWeapon),
      WEAPON_SLOTS,
    ),
    ...bestGroup(
      usable.filter((i) => i.isArmor),
      ARMOR_SLOTS,
    ),
  };
  return summarize(items);
}

/** Power of whatever a character has on right now. */
export function equippedPower(allItems, characterId) {
  const items = {};
  for (const item of allItems) {
    if (
      item.owner === characterId &&
      item.equipped &&
      POWER_SLOTS.includes(item.slot)
    ) {
      items[item.slot] = item;
    }
  }
  return summarize(items);
}

function summarize(items) {
  const missing = POWER_SLOTS.filter((slot) => !items[slot]);
  const sum = POWER_SLOTS.reduce(
    (total, slot) => total + (items[slot]?.power ?? 0),
    0,
  );
  const exactPower = sum / POWER_SLOTS.length;
  return { items, power: Math.floor(exactPower), exactPower, missing };
}
