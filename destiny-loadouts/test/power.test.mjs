import { test } from "node:test";
import assert from "node:assert/strict";
import { equippedPower, maxPowerLoadout, POWER_SLOTS } from "../js/power.js";

let nextId = 1;
const WEAPONS = ["kinetic", "energy", "power"];
const gear = (slot, power, extra = {}) => ({
  id: String(nextId++),
  slot,
  power,
  exotic: false,
  classType: WEAPONS.includes(slot) ? 3 : 1,
  isWeapon: WEAPONS.includes(slot),
  isArmor: !WEAPONS.includes(slot),
  canEquip: true,
  owner: "vault",
  equipped: false,
  ...extra,
});

const baseline = () => POWER_SLOTS.map((slot) => gear(slot, 500));

test("picks the highest Power item in every slot", () => {
  const items = [...baseline(), gear("helmet", 540), gear("kinetic", 530)];
  const result = maxPowerLoadout(items, 1);
  assert.equal(result.items.helmet.power, 540);
  assert.equal(result.items.kinetic.power, 530);
  assert.equal(result.power, Math.floor((500 * 6 + 540 + 530) / 8));
  assert.deepEqual(result.missing, []);
});

test("keeps only one exotic weapon, choosing the best overall", () => {
  const items = [
    ...baseline(),
    gear("kinetic", 550, { exotic: true }),
    gear("energy", 520, { exotic: true }),
    gear("energy", 510),
  ];
  const { items: pick } = maxPowerLoadout(items, 1);
  assert.equal(pick.kinetic.power, 550);
  assert.equal(pick.energy.power, 510);
  assert.equal(pick.energy.exotic, false);
});

test("keeps only one exotic armor piece", () => {
  const items = [
    ...baseline(),
    gear("helmet", 545, { exotic: true }),
    gear("chest", 548, { exotic: true }),
  ];
  const { items: pick } = maxPowerLoadout(items, 1);
  const exotics = ["helmet", "gauntlets", "chest", "legs", "classItem"].filter(
    (s) => pick[s].exotic,
  );
  assert.deepEqual(exotics, ["chest"]);
});

test("an exotic and a weapon exotic can both be used", () => {
  const items = [
    ...baseline(),
    gear("kinetic", 550, { exotic: true }),
    gear("legs", 550, { exotic: true }),
  ];
  const { items: pick } = maxPowerLoadout(items, 1);
  assert.ok(pick.kinetic.exotic && pick.legs.exotic);
});

test("ignores other classes' armor and items that can't be equipped", () => {
  const items = [
    ...baseline(),
    gear("helmet", 550, { classType: 2 }),
    gear("gauntlets", 550, { canEquip: false }),
  ];
  const { items: pick } = maxPowerLoadout(items, 1);
  assert.equal(pick.helmet.power, 500);
  assert.equal(pick.gauntlets.power, 500);
});

test("reports slots with nothing to wear", () => {
  const items = baseline().filter((i) => i.slot !== "power");
  assert.deepEqual(maxPowerLoadout(items, 1).missing, ["power"]);
});

test("an exotic is used when it is the only item for a slot", () => {
  const items = baseline().filter((i) => i.slot !== "legs");
  items.push(gear("legs", 480, { exotic: true }));
  assert.equal(maxPowerLoadout(items, 1).items.legs.power, 480);
});

test("equippedPower averages what the character wears", () => {
  const items = POWER_SLOTS.map((slot, i) =>
    gear(slot, 500 + i, { owner: "A", equipped: true }),
  );
  items.push(gear("helmet", 999, { owner: "A" }));
  assert.equal(equippedPower(items, "A").power, Math.floor((8 * 500 + 28) / 8));
});
