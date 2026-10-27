import { expect, test } from "bun:test";
import {
  coverSeed,
  generatedCover,
  generatedCoverSvg,
} from "./generated-cover";

test("generated covers are deterministic per title and escape text", () => {
  expect(generatedCover("Leaves Fall", "Macroform")).toBe(
    generatedCover("Leaves Fall", "Macroform"),
  );
  expect(coverSeed("a")).not.toBe(coverSeed("b"));
  const svg = generatedCoverSvg('Tom & "Jerry" <live>', "Cat's");
  expect(svg).toContain("Tom &amp; &quot;Jerry&quot; &lt;live&gt;");
  expect(svg).toContain("Cat&apos;s");
  expect(svg).not.toContain("<live>");
  expect(generatedCover("x").startsWith("data:image/svg+xml")).toBe(true);
});

test("generated covers only use brand tones", () => {
  const brand = [
    "#d98b5f",
    "#8fc3e0",
    "#a3c98b",
    "#daab6c",
    "#c9a196",
    "#b8b3aa",
  ];
  for (const title of ["One", "Two", "Three", "Four", "Five", "Six", "Seven"]) {
    const ring = /stroke="(#[0-9a-f]{6})"/.exec(generatedCoverSvg(title))?.[1];
    expect(brand).toContain(ring!);
  }
});
