import { expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { Button } from "./button";

test("default actions use the Mujin Copper primary style", () => {
  const markup = renderToStaticMarkup(<Button>Primary action</Button>);

  expect(markup).toContain("bg-mujin-copper");
  expect(markup).toContain("text-mujin-on-copper");
  expect(markup).toContain("hover:bg-mujin-copper-hover");
  expect(markup).toContain("focus-visible:ring-mujin-copper");
  expect(markup).not.toContain("bg-white");
});

test("outline actions use the shared raised surface", () => {
  const markup = renderToStaticMarkup(
    <Button variant="outline">Secondary action</Button>,
  );

  expect(markup).toContain("rounded-md");
  expect(markup).toContain("bg-zinc-800");
  expect(markup).not.toContain("bg-black");
});
