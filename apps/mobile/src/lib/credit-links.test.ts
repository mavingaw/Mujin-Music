import { expect, test } from "bun:test";
import { artistRoute, joinMeta } from "./credit-links";

test("contributor ids are safe in artist routes", () => {
  expect(artistRoute("mb:561d854a")).toBe("/artist/mb%3A561d854a");
  expect(artistRoute("a/b", "credits")).toBe("/artist/a%2Fb?tab=credits");
});

test("album meta lines skip missing parts", () => {
  expect(joinMeta(["1959", null, "Columbia", undefined, ""])).toBe(
    "1959 · Columbia",
  );
});
