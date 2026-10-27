import { expect, test } from "bun:test";
import type { BackupScheduleStatus } from "@parson/music-sdk";
import { scheduleSummary } from "./schedule-presentation";

const base: BackupScheduleStatus = {
  frequency: "daily",
  destination: null,
  destination_keep: 10,
  destination_report: {
    last_copied_at: null,
    last_copied_file: null,
    last_error: null,
    last_error_at: null,
  },
  local_directory: "/var/lib/parson/Backups",
};

test("the schedule summary tells where backups live and if copies work", () => {
  expect(scheduleSummary(base).text).toContain("/var/lib/parson/Backups");
  expect(scheduleSummary({ ...base, frequency: "off" }).tone).toBe("warn");
  const nas = { ...base, destination: "/mnt/nas" };
  expect(scheduleSummary(nas).text).toContain("next backup");
  expect(
    scheduleSummary({
      ...nas,
      destination_report: {
        ...base.destination_report,
        last_copied_at: "2026-10-26T08:00:00Z",
      },
    }).tone,
  ).toBe("good");
  const failed = scheduleSummary({
    ...nas,
    destination_report: {
      last_copied_at: "2026-10-25T08:00:00Z",
      last_copied_file: "x",
      last_error: "/mnt/nas is not available",
      last_error_at: "2026-10-26T08:00:00Z",
    },
  });
  expect(failed.tone).toBe("warn");
  expect(failed.text).toContain("/mnt/nas is not available");
});
