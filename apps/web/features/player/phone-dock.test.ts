import { describe, expect, test } from "bun:test";
import { holdPhoneDock, nextDockCount } from "./phone-dock";

describe("phone dock", () => {
  test("counts holders and clears when the last one leaves", () => {
    expect(nextDockCount(undefined, 1)).toBe("1");
    expect(nextDockCount("1", 1)).toBe("2");
    expect(nextDockCount("2", -1)).toBe("1");
    expect(nextDockCount("1", -1)).toBe(null);
    expect(nextDockCount("garbage", -1)).toBe(null);
  });

  test("the mini player and the idle bar can overlap during a swap", () => {
    const root = { dataset: {} as DOMStringMap };
    const releaseIdle = holdPhoneDock(root);
    const releaseMini = holdPhoneDock(root);
    expect(root.dataset.phoneDock).toBe("2");
    releaseIdle();
    expect(root.dataset.phoneDock).toBe("1");
    releaseMini();
    expect(root.dataset.phoneDock).toBeUndefined();
  });
});
