import { expect, test } from "bun:test";
import { artistRoute, genreRoute, joinMeta, workRoute } from "./credit-links";

test("contributor ids are safe in artist routes", () => {
  expect(artistRoute("mb:561d854a")).toBe("/artist/mb%3A561d854a");
  expect(artistRoute("a/b", "credits")).toBe("/artist/a%2Fb?tab=credits");
});

test("album meta lines skip missing parts", () => {
  expect(joinMeta(["1959", null, "Columbia", undefined, ""])).toBe(
    "1959 · Columbia",
  );
});

test("work routes encode MusicBrainz ids", () => {
  expect(workRoute("22222222-2222-4222-8222-222222222222")).toBe(
    "/work/22222222-2222-4222-8222-222222222222",
  );
  expect(workRoute("a b")).toBe("/work/a%20b");
});

test("genre routes encode slugs", () => {
  expect(genreRoute("hard-bop")).toBe("/genre/hard-bop");
  expect(genreRoute("neue-deutsche-härte")).toBe(
    "/genre/neue-deutsche-h%C3%A4rte",
  );
});
