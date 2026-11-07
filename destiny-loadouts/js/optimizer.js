// Armor build optimizer. Pure functions: no DOM, no network, so it runs in a
// Web Worker and in Node tests alike.
//
// A "piece" is { id, hash, slot, exotic, stats: number[6], name?, icon? }.
// The six stats are in the order of ARMOR_STAT_HASHES in inventory.js.

export const SLOTS = ["helmet", "gauntlets", "chest", "legs", "classItem"];
export const STAT_COUNT = 6;

/** Weighted score of a stat total, ignoring points above the cap. */
export function scoreTotals(totals, weights, cap) {
  let score = 0;
  for (let i = 0; i < STAT_COUNT; i++) {
    score += weights[i] * Math.min(totals[i], cap);
  }
  return score;
}

function pieceScore(piece, weights) {
  let score = 0;
  for (let i = 0; i < STAT_COUNT; i++) score += weights[i] * piece.stats[i];
  return score;
}

function statSum(piece) {
  let sum = 0;
  for (let i = 0; i < STAT_COUNT; i++) sum += piece.stats[i];
  return sum;
}

/** True when a is at least as good as b in every stat and better in one. */
function dominates(a, b) {
  let better = false;
  for (let i = 0; i < STAT_COUNT; i++) {
    if (a.stats[i] < b.stats[i]) return false;
    if (a.stats[i] > b.stats[i]) better = true;
  }
  return better;
}

function sameStats(a, b) {
  for (let i = 0; i < STAT_COUNT; i++) {
    if (a.stats[i] !== b.stats[i]) return false;
  }
  return true;
}

/**
 * Drops pieces that another piece beats in every stat. Exotics and legendaries
 * are filtered separately, since an exotic is never interchangeable with a
 * legendary. Of several pieces with identical stats only one is kept.
 */
export function paretoFilter(pieces) {
  const keep = [];
  for (const piece of pieces) {
    let dominated = false;
    for (const other of pieces) {
      if (other === piece || other.exotic !== piece.exotic) continue;
      if (dominates(other, piece)) {
        dominated = true;
        break;
      }
    }
    if (dominated) continue;
    if (keep.some((k) => k.exotic === piece.exotic && sameStats(k, piece))) {
      continue;
    }
    keep.push(piece);
  }
  return keep;
}

/**
 * Picks the candidates for one slot: undominated pieces, best first by the
 * weighted score, at most `depth` legendaries and `depth` exotics.
 */
export function candidatesForSlot(pieces, weights, depth) {
  const useTotal = weights.every((w) => w === 0);
  const rank = (p) => (useTotal ? statSum(p) : pieceScore(p, weights));
  const byRank = (a, b) => rank(b) - rank(a) || statSum(b) - statSum(a);
  const filtered = paretoFilter(pieces).sort(byRank);
  const legendary = filtered.filter((p) => !p.exotic).slice(0, depth);
  const exotic = filtered.filter((p) => p.exotic).slice(0, depth);
  return [...legendary, ...exotic];
}

/**
 * Finds the best armor sets.
 *
 * options:
 *   weights  number[6]  how much each stat point is worth (0 = ignore)
 *   minimums number[6]  stat totals a build must reach
 *   cap      number     points above this per stat are wasted (Armor 3.0: 200)
 *   exotic   "any" | "none" | number   "any" allows at most one exotic,
 *                       "none" forbids exotics, a number is the exotic's item
 *                       hash that every build must include
 *   depth    number     candidates kept per slot (search breadth)
 *   limit    number     how many builds to return
 *
 * Returns { builds, combinationsChecked }, builds best first. Each build is
 * { pieces: piece[5] in SLOTS order, totals: number[6], score, wasted }.
 */
export function optimize(piecesBySlot, options) {
  const {
    weights = [1, 1, 1, 1, 1, 1],
    minimums = [0, 0, 0, 0, 0, 0],
    cap = 200,
    exotic = "any",
    depth = 14,
    limit = 20,
  } = options;

  const useTotal = weights.every((w) => w === 0);
  const effectiveWeights = useTotal ? [1, 1, 1, 1, 1, 1] : weights;

  const lists = SLOTS.map((slot) => {
    let pieces = piecesBySlot[slot] ?? [];
    if (exotic === "none") {
      pieces = pieces.filter((p) => !p.exotic);
    } else if (typeof exotic === "number") {
      const hasLocked = pieces.some((p) => p.hash === exotic);
      // The locked exotic's slot may only hold that exotic; the rest of the
      // slots hold legendaries.
      pieces = hasLocked
        ? pieces.filter((p) => p.hash === exotic)
        : pieces.filter((p) => !p.exotic);
    }
    return candidatesForSlot(pieces, effectiveWeights, depth);
  });

  if (lists.some((l) => l.length === 0)) {
    return { builds: [], combinationsChecked: 0 };
  }

  const best = [];
  let worstKept = -Infinity;
  let combinationsChecked = 0;
  const totals = [0, 0, 0, 0, 0, 0];
  const chosen = new Array(SLOTS.length);

  const consider = () => {
    combinationsChecked++;
    for (let i = 0; i < STAT_COUNT; i++) {
      if (totals[i] < minimums[i]) return;
    }
    const score = scoreTotals(totals, effectiveWeights, cap);
    if (best.length >= limit && score <= worstKept) return;

    let wasted = 0;
    let capped = 0;
    for (let i = 0; i < STAT_COUNT; i++) {
      wasted += Math.max(0, totals[i] - cap);
      capped += Math.min(totals[i], cap);
    }
    const build = {
      pieces: chosen.slice(),
      totals: totals.slice(),
      score,
      wasted,
      capped,
    };
    let at = best.findIndex(
      (b) => score > b.score || (score === b.score && capped > b.capped),
    );
    if (at === -1) at = best.length;
    best.splice(at, 0, build);
    if (best.length > limit) best.pop();
    if (best.length >= limit) worstKept = best[best.length - 1].score;
  };

  const walk = (slotIndex, exoticCount) => {
    if (slotIndex === SLOTS.length) {
      if (typeof exotic === "number" && exoticCount !== 1) return;
      consider();
      return;
    }
    for (const piece of lists[slotIndex]) {
      const nextExotics = exoticCount + (piece.exotic ? 1 : 0);
      if (nextExotics > 1) continue;
      for (let i = 0; i < STAT_COUNT; i++) totals[i] += piece.stats[i];
      chosen[slotIndex] = piece;
      walk(slotIndex + 1, nextExotics);
      for (let i = 0; i < STAT_COUNT; i++) totals[i] -= piece.stats[i];
    }
  };

  walk(0, 0);
  return { builds: best, combinationsChecked };
}
