import { expect, test } from "bun:test";
import {
  backupDetail,
  backupTitle,
  formatBytes,
  readServerBootId,
  waitForRestart,
} from "./migration-presentation";

const backup = {
  filename: "parson-backup-20261020T080000000Z.tar.zst",
  path: "/mnt/nas/parson-backup-20261020T080000000Z.tar.zst",
  size_bytes: 1_258_291,
  created_at: "2026-10-20T08:00:00+00:00",
  library_name: "Home Library",
  instance_id: "33333333-3333-4333-8333-333333333333",
  product_version: "1.0.1",
  library_roots: ["/home/ana/Music"],
};

test("backups describe where they came from", () => {
  expect(backupTitle(backup)).toMatch(/^Home Library · 20 Oct 2026/);
  expect(backupDetail(backup)).toBe(
    "1.2 MB · Parson 1.0.1 · music was in /home/ana/Music",
  );
  expect(backupTitle({ ...backup, library_name: null, created_at: null })).toBe(
    "Parson library",
  );
  expect(formatBytes(0)).toBe("0 B");
});

test("a restart is detected by a new boot id after the server went away", async () => {
  let clock = 0;
  const answers: (string | Error)[] = [
    "boot-1",
    new Error("down"),
    new Error("down"),
    "boot-2",
  ];
  const restarted = await waitForRestart({
    previousBootId: "boot-1",
    readBootId: async () => {
      const next = answers.shift() ?? "boot-2";
      if (next instanceof Error) throw next;
      return next;
    },
    sleep: async () => {
      clock += 1_000;
    },
    now: () => clock,
  });
  expect(restarted).toBe(true);
  expect(answers).toEqual([]);
});

test("waiting for a restart gives up after the timeout", async () => {
  let clock = 0;
  const restarted = await waitForRestart({
    previousBootId: "boot-1",
    readBootId: async () => "boot-1",
    timeoutMs: 5_000,
    sleep: async () => {
      clock += 1_000;
    },
    now: () => clock,
  });
  expect(restarted).toBe(false);
});

test("the boot id is read from the public manifest", async () => {
  const fetcher = (async () =>
    new Response(JSON.stringify({ bootId: "abc" }), {
      status: 200,
    })) as unknown as typeof fetch;
  expect(await readServerBootId(fetcher)).toBe("abc");
  const failing = (async () =>
    new Response("", { status: 503 })) as unknown as typeof fetch;
  await expect(readServerBootId(failing)).rejects.toThrow("HTTP 503");
});
