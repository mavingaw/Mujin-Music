import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  getNativeOutputSession,
  getNativeOutputs,
  nativeFormatLabel,
  playOnNativeOutput,
  refreshNativeOutputs,
  sendNativeOutputCommand,
  updateNativeOutputSettings,
} from "./outputs";

afterEach(() => mock.restore());

const ok = (data: unknown) =>
  ({ data, status: 200, headers: new Headers() }) as never;

test("lists and refreshes the server's devices", async () => {
  const view = {
    supported: true,
    driver: "alsa",
    simulated: false,
    devices: [],
  };
  const get = spyOn(api, "get").mockResolvedValue(ok(view));
  const post = spyOn(api, "post").mockResolvedValue(ok(view));
  expect(await getNativeOutputs()).toBe(view as never);
  expect(get).toHaveBeenCalledWith("/outputs/native");
  await refreshNativeOutputs();
  expect(post).toHaveBeenCalledWith("/outputs/native/admin/refresh", {});
});

test("saves a device's exclusive mode and format caps", async () => {
  const put = spyOn(api, "put").mockResolvedValue(ok({ id: "alsa:USB:0" }));
  await updateNativeOutputSettings("alsa:USB:0", {
    exclusive: true,
    max_sample_rate: 192_000,
    max_bit_depth: 24,
  });
  expect(put).toHaveBeenCalledWith("/outputs/native/admin/settings", {
    device_id: "alsa:USB:0",
    settings: { exclusive: true, max_sample_rate: 192_000, max_bit_depth: 24 },
  });
});

test("plays, controls and stops the native session", async () => {
  const session = { device_id: "alsa:USB:0", state: "playing" };
  const post = spyOn(api, "post").mockResolvedValue(ok(session));
  const get = spyOn(api, "get").mockResolvedValue(ok(null));
  expect(await getNativeOutputSession()).toBeNull();
  expect(get).toHaveBeenCalledWith("/outputs/native/session");
  await playOnNativeOutput({
    device_id: "alsa:USB:0",
    song_id: "song-1",
    position_ms: 1234.6,
  });
  expect(post).toHaveBeenLastCalledWith("/outputs/native/session", {
    device_id: "alsa:USB:0",
    song_id: "song-1",
    position_ms: 1235,
  });
  await playOnNativeOutput({
    device_id: "alsa:USB:0",
    song_id: "song-1",
    leveling: { gain_db: -6.4, source: "album" },
  });
  expect(post).toHaveBeenLastCalledWith("/outputs/native/session", {
    device_id: "alsa:USB:0",
    song_id: "song-1",
    position_ms: 0,
    leveling: { gain_db: -6.4, source: "album" },
  });
  expect(await sendNativeOutputCommand("seek", -5)).toBe(session as never);
  expect(post).toHaveBeenLastCalledWith("/outputs/native/session/commands", {
    command: "seek",
    position_ms: 0,
  });
  expect(await sendNativeOutputCommand("stop")).toBeNull();
  expect(post).toHaveBeenLastCalledWith("/outputs/native/session/commands", {
    command: "stop",
  });
});

test("labels rates like the format support list", () => {
  expect(nativeFormatLabel(44_100)).toBe("44.1kHz");
  expect(nativeFormatLabel(192_000)).toBe("192kHz");
});
