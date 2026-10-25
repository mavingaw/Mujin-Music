import { describe, expect, test } from "bun:test";
import { isDspShortcut } from "./dsp-panel";

const key = (overrides: Partial<KeyboardEvent>) => ({
  code: "Slash",
  key: "/",
  ctrlKey: false,
  metaKey: false,
  altKey: false,
  shiftKey: false,
  ...overrides,
});

describe("DSP engine shortcut", () => {
  test("Ctrl+/ and Cmd+/ open it, like Roon's MUSE", () => {
    expect(isDspShortcut(key({ ctrlKey: true }))).toBeTrue();
    expect(isDspShortcut(key({ metaKey: true }))).toBeTrue();
    // Keyboard layouts that put "/" elsewhere still match by key.
    expect(
      isDspShortcut(key({ ctrlKey: true, code: "Digit7", key: "/" })),
    ).toBeTrue();
  });

  test("plain or shifted slashes do not", () => {
    expect(isDspShortcut(key({}))).toBeFalse();
    expect(isDspShortcut(key({ ctrlKey: true, shiftKey: true }))).toBeFalse();
    expect(isDspShortcut(key({ ctrlKey: true, altKey: true }))).toBeFalse();
    expect(
      isDspShortcut(key({ ctrlKey: true, code: "KeyK", key: "k" })),
    ).toBeFalse();
  });
});
