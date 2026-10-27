import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { RadioSwitch } from "./radio-controls";

describe("the shared on/off switch", () => {
  test("is Mujin Copper when on", () => {
    const markup = renderToStaticMarkup(
      <RadioSwitch checked label="Radio" onChange={() => {}} />,
    );

    expect(markup).toContain('aria-checked="true"');
    expect(markup).toContain("bg-mujin-copper");
    expect(markup).toContain("border-mujin-copper-hover");
    expect(markup).not.toContain("sky-");
  });

  test("is a neutral track when off", () => {
    const markup = renderToStaticMarkup(
      <RadioSwitch checked={false} label="Radio" onChange={() => {}} />,
    );

    expect(markup).toContain('aria-checked="false"');
    expect(markup).not.toContain("mujin-copper");
    expect(markup).not.toContain("sky-");
  });
});
