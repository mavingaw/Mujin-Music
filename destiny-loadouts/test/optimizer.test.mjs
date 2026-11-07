import { test } from "node:test";
import assert from "node:assert/strict";
import { optimize, paretoFilter, scoreTotals, SLOTS } from "../js/optimizer.js";

let nextId = 1;
const piece = (slot, stats, exotic = false, hash = 0) => ({
  id: String(nextId++),
  hash,
  slot,
  exotic,
  stats,
});

const flat = (n) => [n, n, n, n, n, n];

function wardrobe(extra = {}) {
  const set = {};
  for (const slot of SLOTS) set[slot] = [piece(slot, flat(10))];
  for (const [slot, pieces] of Object.entries(extra)) {
    set[slot].push(...pieces);
  }
  return set;
}

test("scoreTotals ignores points above the cap", () => {
  assert.equal(scoreTotals([250, 0, 0, 0, 0, 0], flat(1), 200), 200);
  assert.equal(scoreTotals([10, 20, 0, 0, 0, 0], [2, 1, 0, 0, 0, 0], 200), 40);
});

test("paretoFilter drops dominated pieces and duplicates", () => {
  const strong = piece("helmet", [30, 10, 10, 10, 10, 10]);
  const weak = piece("helmet", [20, 10, 10, 10, 10, 10]);
  const copy = piece("helmet", [30, 10, 10, 10, 10, 10]);
  const different = piece("helmet", [0, 40, 0, 0, 0, 0]);
  const kept = paretoFilter([strong, weak, copy, different]);
  assert.deepEqual(kept, [strong, different]);
});

test("paretoFilter never lets a legendary knock out an exotic", () => {
  const legendary = piece("helmet", flat(30));
  const exotic = piece("helmet", flat(5), true);
  assert.equal(paretoFilter([legendary, exotic]).length, 2);
});

test("picks the piece with the most of the weighted stat", () => {
  const weapons = piece("helmet", [40, 0, 0, 0, 0, 0]);
  const health = piece("helmet", [0, 40, 0, 0, 0, 0]);
  const { builds } = optimize(wardrobe({ helmet: [weapons, health] }), {
    weights: [0, 5, 0, 0, 0, 0],
  });
  assert.equal(builds[0].pieces[0], health);
  assert.equal(builds[0].totals[1], 80);
});

test("all-zero weights maximise the total of all stats", () => {
  const big = piece("legs", [20, 20, 20, 20, 20, 20]);
  const { builds } = optimize(wardrobe({ legs: [big] }), {
    weights: flat(0),
  });
  assert.equal(builds[0].pieces[3], big);
});

test("never builds with two exotics", () => {
  const exoHelm = piece("helmet", flat(50), true, 111);
  const exoChest = piece("chest", flat(50), true, 222);
  const { builds } = optimize(
    wardrobe({ helmet: [exoHelm], chest: [exoChest] }),
    { limit: 50 },
  );
  for (const build of builds) {
    assert.ok(build.pieces.filter((p) => p.exotic).length <= 1);
  }
});

test("a locked exotic is in every build", () => {
  const exoHelm = piece("helmet", flat(50), true, 111);
  const exoChest = piece("chest", flat(5), true, 222);
  const { builds } = optimize(
    wardrobe({ helmet: [exoHelm], chest: [exoChest] }),
    { exotic: 222 },
  );
  assert.ok(builds.length > 0);
  for (const build of builds) {
    assert.equal(build.pieces[2], exoChest);
    assert.equal(build.pieces.filter((p) => p.exotic).length, 1);
  }
});

test("'none' keeps exotics out", () => {
  const exoHelm = piece("helmet", flat(50), true, 111);
  const { builds } = optimize(wardrobe({ helmet: [exoHelm] }), {
    exotic: "none",
  });
  assert.ok(builds.every((b) => b.pieces.every((p) => !p.exotic)));
});

test("minimums filter out builds that fall short", () => {
  const melee = piece("gauntlets", [0, 0, 0, 0, 0, 60]);
  const sets = wardrobe({ gauntlets: [melee] });
  const { builds } = optimize(sets, {
    weights: [1, 0, 0, 0, 0, 0],
    minimums: [0, 0, 0, 0, 0, 100],
  });
  assert.ok(builds.length > 0);
  for (const build of builds) assert.ok(build.totals[5] >= 100);

  const none = optimize(sets, { minimums: [0, 0, 0, 0, 0, 500] });
  assert.equal(none.builds.length, 0);
});

test("the cap stops one stat from swamping the score", () => {
  const huge = piece("helmet", [300, 0, 0, 0, 0, 0]);
  const balanced = piece("helmet", [30, 30, 30, 30, 30, 30]);
  const { builds } = optimize(wardrobe({ helmet: [huge, balanced] }), {
    weights: flat(1),
    cap: 200,
  });
  assert.equal(builds[0].pieces[0], balanced);
  const hugeBuild = builds.find((b) => b.pieces[0] === huge);
  assert.ok(hugeBuild.wasted > 0);
});

test("returns builds best first and respects the limit", () => {
  const sets = {};
  for (const slot of SLOTS) {
    sets[slot] = Array.from({ length: 6 }, (_, i) =>
      piece(slot, [i * 5, 30 - i * 5, 10, 10, 10, 10]),
    );
  }
  const { builds, combinationsChecked } = optimize(sets, {
    weights: [3, 1, 0, 0, 0, 0],
    limit: 7,
  });
  assert.equal(builds.length, 7);
  assert.equal(combinationsChecked, 6 ** 5);
  for (let i = 1; i < builds.length; i++) {
    assert.ok(builds[i - 1].score >= builds[i].score);
  }
});

test("an empty slot means no builds", () => {
  const sets = wardrobe();
  sets.classItem = [];
  assert.equal(optimize(sets, {}).builds.length, 0);
});
