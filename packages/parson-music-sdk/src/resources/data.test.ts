import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  restoreBackupFromFolder,
  restoreQuery,
  restoreSetupBackupFromFolder,
} from "./data";

afterEach(() => mock.restore());

test("a restore brings the away-from-home setting only when asked", async () => {
  expect(restoreQuery()).toBe("");
  expect(restoreQuery({ adoptIdentity: true })).toBe("?adopt_identity=true");
  expect(restoreQuery({ adoptIdentity: true, restoreRemoteAccess: true })).toBe(
    "?adopt_identity=true&restore_remote_access=true",
  );

  const post = spyOn(api, "post").mockResolvedValue({
    data: { restart_required: false, restarting: true, message: "ok" },
    status: 202,
    headers: new Headers(),
  } as never);
  await restoreSetupBackupFromFolder("/mnt/nas/b.tar.zst", {
    adoptIdentity: true,
  });
  await restoreBackupFromFolder("/mnt/nas/b.tar.zst", {
    restoreRemoteAccess: true,
  });
  expect(post.mock.calls[0][1]).toEqual({
    path: "/mnt/nas/b.tar.zst",
    adopt_identity: true,
    restore_remote_access: false,
  });
  expect(post.mock.calls[1][1]).toEqual({
    path: "/mnt/nas/b.tar.zst",
    adopt_identity: false,
    restore_remote_access: true,
  });
});
