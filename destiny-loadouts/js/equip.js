// Moving and equipping items, the way DIM does it: items on other characters
// go through the vault, a full slot makes room by sending a spare item to the
// vault, and exotics are equipped last so they never clash with the exotic
// being replaced.

import { api } from "./bungie.js";
import { BUCKET_CAPACITY } from "./inventory.js";

/**
 * Works out the steps to equip `itemIds` on `characterId`, without touching
 * the network. `items` is the inventory list from buildInventory.
 *
 * Returns { steps, problems }. Each step is one of
 *   { kind: "transfer", item, characterId, toVault }
 *   { kind: "equip", characterId, items }
 * problems lists items that cannot be moved, with the reason.
 */
export function planEquip(items, characterId, itemIds) {
  // Work on copies so planning can simulate the moves.
  const sim = items.map((item) => ({ ...item }));
  const byId = new Map(sim.map((item) => [item.id, item]));
  const wanted = new Set(itemIds);
  const steps = [];
  const problems = [];

  const inBucket = (owner, bucket) =>
    sim.filter((i) => i.owner === owner && i.bucket === bucket);

  const transfer = (item, toVault, charId) => {
    steps.push({ kind: "transfer", item, characterId: charId, toVault });
    item.owner = toVault ? "vault" : charId;
    item.equipped = false;
  };

  for (const id of itemIds) {
    const item = byId.get(id);
    if (!item) {
      problems.push({
        id,
        reason: "This item is no longer in your inventory.",
      });
      continue;
    }
    if (item.owner === characterId) continue;

    // An item equipped on another character has to be swapped out first.
    if (item.owner !== "vault" && item.equipped) {
      const replacement = inBucket(item.owner, item.bucket)
        .filter((i) => !i.equipped && !wanted.has(i.id) && i.canEquip)
        .sort((a, b) => Number(a.exotic) - Number(b.exotic))[0];
      if (!replacement) {
        problems.push({
          id,
          item,
          reason:
            "It is equipped on another character with nothing to swap in. Unequip it in game first.",
        });
        continue;
      }
      steps.push({
        kind: "equip",
        characterId: item.owner,
        items: [replacement],
      });
      replacement.equipped = true;
      item.equipped = false;
    }

    if (item.owner !== "vault") transfer(item, true, item.owner);

    // Make room on the target character if that slot is full.
    const spares = inBucket(characterId, item.bucket).filter(
      (i) => !i.equipped,
    );
    if (spares.length >= BUCKET_CAPACITY) {
      const filler = spares
        .filter((i) => !wanted.has(i.id))
        .sort((a, b) => Number(a.locked) - Number(b.locked))[0];
      if (!filler) {
        problems.push({
          id,
          item,
          reason:
            "That slot is full on this character and nothing can be moved out.",
        });
        continue;
      }
      transfer(filler, true, characterId);
    }

    transfer(item, false, characterId);
  }

  const toEquip = itemIds
    .map((id) => byId.get(id))
    .filter(
      (item) =>
        item &&
        item.owner === characterId &&
        !item.equipped &&
        !problems.some((p) => p.id === item.id),
    );
  const legendary = toEquip.filter((i) => !i.exotic);
  const exotic = toEquip.filter((i) => i.exotic);
  if (legendary.length) {
    steps.push({ kind: "equip", characterId, items: legendary });
  }
  if (exotic.length) steps.push({ kind: "equip", characterId, items: exotic });

  return { steps, problems };
}

const EQUIP_FAILURES = {
  // PlatformErrorCodes the game returns for common equip failures.
  1623: "The game could not find this item. Refresh and try again.",
  1634: "That character is in an activity. Go to orbit or a social space first.",
  1641: "You can only equip one exotic of this kind.",
  1656: "That item is equipped, so it can't be moved right now.",
  1671: "Items can't be moved or equipped where that character is right now.",
};

/**
 * Runs the plan against Bungie.net. onProgress gets a short line per step.
 * Returns { failures: [{ item, reason }] }.
 */
export async function runPlan(plan, membershipType, onProgress = () => {}) {
  const failures = plan.problems.map((p) => ({
    item: p.item,
    reason: p.reason,
  }));
  for (const step of plan.steps) {
    if (step.kind === "transfer") {
      onProgress(
        step.toVault
          ? `Moving ${step.item.name} to the vault…`
          : `Moving ${step.item.name} to your character…`,
      );
      try {
        await api("/Destiny2/Actions/Items/TransferItem/", {
          method: "POST",
          body: {
            itemReferenceHash: step.item.hash,
            stackSize: 1,
            transferToVault: step.toVault,
            itemId: step.item.id,
            characterId: step.characterId,
            membershipType,
          },
        });
      } catch (error) {
        failures.push({ item: step.item, reason: error.message });
      }
      continue;
    }

    onProgress(`Equipping ${step.items.map((i) => i.name).join(", ")}…`);
    try {
      const result = await api("/Destiny2/Actions/Items/EquipItems/", {
        method: "POST",
        body: {
          itemIds: step.items.map((i) => i.id),
          characterId: step.characterId,
          membershipType,
        },
      });
      for (const r of result?.equipResults ?? []) {
        if (r.equipStatus === 1) continue;
        const item = step.items.find((i) => i.id === r.itemInstanceId);
        failures.push({
          item,
          reason:
            EQUIP_FAILURES[r.equipStatus] ??
            `Equip failed (code ${r.equipStatus}).`,
        });
      }
    } catch (error) {
      for (const item of step.items)
        failures.push({ item, reason: error.message });
    }
  }
  return { failures };
}

/** Equips one of the character's in-game loadouts (the ones saved in game). */
export function equipGameLoadout(loadoutIndex, characterId, membershipType) {
  return api("/Destiny2/Actions/Loadouts/EquipLoadout/", {
    method: "POST",
    body: { loadoutIndex, characterId, membershipType },
  });
}
