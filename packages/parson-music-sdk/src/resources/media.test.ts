import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import { getSongDelivery } from "./media";

afterEach(() => {
  mock.restore();
});

test("delivery requests carry the player's capabilities", async () => {
  const get = spyOn(api, "get").mockResolvedValue({
    data: { song_id: "a/b", plan: { mode: "original" } },
    status: 200,
    headers: new Headers(),
  } as never);

  const delivery = await getSongDelivery(
    "a/b",
    { accept: ["flac", "opus"], maxSampleRate: 192_000 },
    { bitrate: 0, audioEffect: "none" },
  );

  expect(delivery.plan.mode).toBe("original");
  expect(get).toHaveBeenCalledWith("/media/songs/a%2Fb/delivery", {
    params: { accept: "flac,opus", max_sample_rate: "192000", bitrate: "0" },
    timeout: 8_000,
  });
});
