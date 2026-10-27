import { expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import AboutSettings, { ATTRIBUTION } from "./about-settings";

test("About names Mujin Music and credits Parson by ParsonLabs", () => {
  const markup = renderToStaticMarkup(<AboutSettings />);
  expect(ATTRIBUTION).toBe("Based on Parson by ParsonLabs");
  expect(markup).toContain("Mujin Music");
  expect(markup).toContain("Based on Parson by ParsonLabs");
  expect(markup).toContain("GNU General Public License");
  expect(markup).toContain("https://github.com/ParsonLabs/Parson");
  expect(markup).toContain('class="mj-signal"');
});
