import { describe, expect, test } from "bun:test";
import { remoteRoleCopy, serviceSummary } from "./desktop-service";

describe("serviceSummary", () => {
  test("names each platform's background mechanism", () => {
    expect(
      serviceSummary({
        kind: "launch-agent",
        available: true,
        running: true,
        autostart: true,
      }),
    ).toEqual({
      running: true,
      status: "Running in the background",
      detail: "Parson Server in the menu bar",
      autostartLabel: "Launch at startup",
    });
    expect(
      serviceSummary({ kind: "windows-tray", available: true, running: true })
        ?.autostartLabel,
    ).toBe("Start with Windows");
    expect(
      serviceSummary({ kind: "systemd-user", available: true, running: true })
        ?.detail,
    ).toContain("systemd");
  });

  test("is honest when the server is stopped", () => {
    const summary = serviceSummary({
      kind: "systemd-user",
      available: true,
      running: false,
    });
    expect(summary?.running).toBe(false);
    expect(summary?.detail).toContain("can’t reach your library");
  });

  test("hides itself without a service", () => {
    expect(serviceSummary(null)).toBeNull();
    expect(serviceSummary({ kind: null, available: false })).toBeNull();
  });
});

test("the remote role card never hides this computer's own server", () => {
  expect(remoteRoleCopy({ name: "Studio NAS", localRunning: true })).toContain(
    "This computer also runs its own Parson server",
  );
  expect(remoteRoleCopy({ name: "Studio NAS", localRunning: false })).toMatch(
    /^No server runs on this computer/,
  );
  expect(remoteRoleCopy({ name: "Studio NAS" })).toMatch(/^No server runs/);
});
