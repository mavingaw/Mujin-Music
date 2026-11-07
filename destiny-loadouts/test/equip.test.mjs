import { test } from "node:test";
import assert from "node:assert/strict";
import { planEquip } from "../js/equip.js";
import { BUCKETS } from "../js/inventory.js";

let nextId = 1;
const item = (owner, bucket, extra = {}) => ({
  id: String(nextId++),
  hash: 1000 + nextId,
  name: `Item ${nextId}`,
  bucket,
  owner,
  equipped: false,
  exotic: false,
  locked: false,
  canEquip: true,
  ...extra,
});

const kinds = (plan) =>
  plan.steps.map((s) =>
    s.kind === "equip"
      ? `equip:${s.characterId}:${s.items.map((i) => i.id).join(",")}`
      : `${s.toVault ? "vault" : "char"}:${s.item.id}`,
  );

test("items already on the character are just equipped", () => {
  const helm = item("A", BUCKETS.helmet);
  const plan = planEquip([helm], "A", [helm.id]);
  assert.deepEqual(kinds(plan), [`equip:A:${helm.id}`]);
});

test("items already equipped need no steps", () => {
  const helm = item("A", BUCKETS.helmet, { equipped: true });
  assert.deepEqual(planEquip([helm], "A", [helm.id]).steps, []);
});

test("vault items move to the character, then equip", () => {
  const helm = item("vault", BUCKETS.helmet);
  const plan = planEquip([helm], "A", [helm.id]);
  assert.deepEqual(kinds(plan), [`char:${helm.id}`, `equip:A:${helm.id}`]);
});

test("items on another character go through the vault", () => {
  const helm = item("B", BUCKETS.helmet);
  const plan = planEquip([helm], "A", [helm.id]);
  assert.deepEqual(kinds(plan), [
    `vault:${helm.id}`,
    `char:${helm.id}`,
    `equip:A:${helm.id}`,
  ]);
});

test("an item equipped elsewhere is swapped out first", () => {
  const helm = item("B", BUCKETS.helmet, { equipped: true });
  const spareExotic = item("B", BUCKETS.helmet, { exotic: true });
  const spare = item("B", BUCKETS.helmet);
  const plan = planEquip([helm, spareExotic, spare], "A", [helm.id]);
  // The legendary spare is preferred so it can't clash with B's exotic.
  assert.deepEqual(kinds(plan), [
    `equip:B:${spare.id}`,
    `vault:${helm.id}`,
    `char:${helm.id}`,
    `equip:A:${helm.id}`,
  ]);
});

test("reports an item equipped elsewhere with nothing to swap in", () => {
  const helm = item("B", BUCKETS.helmet, { equipped: true });
  const plan = planEquip([helm], "A", [helm.id]);
  assert.equal(plan.steps.length, 0);
  assert.equal(plan.problems.length, 1);
});

test("a full slot sends an unlocked spare to the vault first", () => {
  const spares = Array.from({ length: 9 }, (_, i) =>
    item("A", BUCKETS.legs, { locked: i !== 4 }),
  );
  const legs = item("vault", BUCKETS.legs);
  const plan = planEquip([...spares, legs], "A", [legs.id]);
  assert.deepEqual(kinds(plan), [
    `vault:${spares[4].id}`,
    `char:${legs.id}`,
    `equip:A:${legs.id}`,
  ]);
});

test("exotics are equipped after legendaries", () => {
  const exoHelm = item("A", BUCKETS.helmet, { exotic: true });
  const chest = item("A", BUCKETS.chest);
  const plan = planEquip([exoHelm, chest], "A", [exoHelm.id, chest.id]);
  assert.deepEqual(kinds(plan), [
    `equip:A:${chest.id}`,
    `equip:A:${exoHelm.id}`,
  ]);
});

test("planning leaves the real inventory untouched", () => {
  const helm = item("B", BUCKETS.helmet);
  const items = [helm];
  planEquip(items, "A", [helm.id]);
  assert.equal(helm.owner, "B");
});

test("missing items are reported, not thrown", () => {
  const plan = planEquip([], "A", ["nope"]);
  assert.equal(plan.problems.length, 1);
});
