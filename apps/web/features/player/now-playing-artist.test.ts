import { expect, test } from "bun:test";
import { bioExcerpt } from "./now-playing-artist";

test("a short biography is shown whole", () => {
  expect(bioExcerpt("Nine Inch Nails is an American band.")).toBe(
    "Nine Inch Nails is an American band.",
  );
});

test("a long biography is cut at a sentence, then at a word", () => {
  const body =
    "Nine Inch Nails is an American industrial rock band formed in Cleveland, Ohio, in 1988. " +
    "Its members are Trent Reznor and Atticus Ross. Reznor was the only official member until 2016, " +
    "when Ross became a permanent member. The project has released many albums over the years.";
  const excerpt = bioExcerpt(body, 160);
  expect(excerpt.endsWith(".")).toBe(true);
  expect(excerpt.length).toBeLessThanOrEqual(160);
  const words = bioExcerpt("word ".repeat(100), 50);
  expect(words.endsWith("…")).toBe(true);
  expect(words.length).toBeLessThanOrEqual(51);
});

test("only the first paragraph is used", () => {
  expect(bioExcerpt("First paragraph.\n\nSecond paragraph.")).toBe(
    "First paragraph.",
  );
});
