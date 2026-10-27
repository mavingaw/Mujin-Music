import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import { indexSetupLibrary } from "./library";

afterEach(() => {
  mock.restore();
});

test("first-run setup sends the online metadata choice with the folder", async () => {
  const post = spyOn(api, "post").mockResolvedValue({
    data: { library: null, report: {} },
    status: 200,
    headers: new Headers(),
  } as never);
  await indexSetupLibrary("/music", { onlineMetadata: true });
  await indexSetupLibrary("/music", { onlineMetadata: false });
  await indexSetupLibrary("/music");
  expect(post.mock.calls[0]?.[0]).toBe("/setup/library");
  expect(post.mock.calls[0]?.[1]).toEqual({
    path: "/music",
    online_metadata: true,
  });
  expect(post.mock.calls[1]?.[1]).toEqual({
    path: "/music",
    online_metadata: false,
  });
  // Older callers send only the folder; the server keeps its setting.
  expect(post.mock.calls[2]?.[1]).toEqual({ path: "/music" });
});
