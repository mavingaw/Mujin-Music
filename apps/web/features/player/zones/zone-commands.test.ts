import { describe, expect, test } from "bun:test";
import {
  applyZoneCommand,
  heartbeatInterval,
  localZoneStatus,
  queueKey,
  type LocalZoneControls,
} from "./zone-commands";
import {
  clientZoneIdentity,
  describeBrowser,
  localZoneForHistory,
  readOrCreateDeviceId,
} from "./zone-device";

function fakeControls(playing = false, muted = false) {
  const calls: string[] = [];
  const controls: LocalZoneControls = {
    isPlaying: () => playing,
    isMuted: () => muted,
    play: () => calls.push("play"),
    pause: () => calls.push("pause"),
    next: () => calls.push("next"),
    previous: () => calls.push("previous"),
    seek: (seconds) => calls.push(`seek:${seconds}`),
    jump: (index) => calls.push(`jump:${index}`),
    setVolume: (percent) => calls.push(`volume:${percent}`),
    setMuted: (value) => calls.push(`muted:${value}`),
    load: async (ids, position, positionMs, play) => {
      calls.push(`load:${ids.join(",")}@${position}+${positionMs}:${play}`);
    },
    songIdAt: (index) => ["a", "b", "c", "d"][index],
    moveQueueItem: (from, to) => calls.push(`move:${from}->${to}`),
    removeQueueItem: (index) => calls.push(`remove:${index}`),
    clearUpcoming: () => calls.push("clear"),
    addSongs: async (ids, next) => {
      calls.push(`add:${ids.join(",")}${next ? ":next" : ""}`);
    },
    setRepeat: (mode) => calls.push(`repeat:${mode}`),
    setShuffle: (enabled) => calls.push(`shuffle:${enabled}`),
  };
  return { calls, controls };
}

describe("remote zone commands on this device", () => {
  test("queue edits apply only to the queue the remote saw", async () => {
    const { calls, controls } = fakeControls(true);
    await applyZoneCommand(
      { command: "move_queue_item", from: 3, to: 2, song_id: "d" },
      controls,
    );
    await applyZoneCommand(
      { command: "move_queue_item", from: 3, to: 2, song_id: "x" },
      controls,
    );
    await applyZoneCommand(
      { command: "remove_queue_item", queue_position: 2, song_id: "c" },
      controls,
    );
    await applyZoneCommand(
      { command: "remove_queue_item", queue_position: 2, song_id: "z" },
      controls,
    );
    await applyZoneCommand({ command: "clear_upcoming" }, controls);
    await applyZoneCommand({ command: "set_repeat", mode: "all" }, controls);
    await applyZoneCommand({ command: "set_shuffle", enabled: true }, controls);
    expect(calls).toEqual([
      "move:3->2",
      "remove:2",
      "clear",
      "repeat:all",
      "shuffle:true",
    ]);
  });

  test("play next and add to queue reach the local queue", async () => {
    const { calls, controls } = fakeControls(true);
    await applyZoneCommand(
      { command: "add_to_queue", song_ids: ["x", "y"], next: true },
      controls,
    );
    await applyZoneCommand(
      { command: "add_to_queue", song_ids: ["z"] },
      controls,
    );
    await applyZoneCommand(
      { command: "add_to_queue", song_ids: [], next: true },
      controls,
    );
    expect(calls).toEqual(["add:x,y:next", "add:z"]);
  });

  test("play and pause are idempotent", async () => {
    const paused = fakeControls(false);
    await applyZoneCommand({ command: "play" }, paused.controls);
    await applyZoneCommand({ command: "pause" }, paused.controls);
    expect(paused.calls).toEqual(["play"]);
    const playing = fakeControls(true);
    await applyZoneCommand({ command: "play" }, playing.controls);
    await applyZoneCommand({ command: "stop" }, playing.controls);
    await applyZoneCommand({ command: "play_pause" }, playing.controls);
    expect(playing.calls).toEqual(["pause", "pause"]);
  });

  test("seek, volume, mute, jump and load map onto the player", async () => {
    const { calls, controls } = fakeControls(true, false);
    await applyZoneCommand({ command: "seek", position_ms: 42_500 }, controls);
    await applyZoneCommand({ command: "set_volume", volume: 140 }, controls);
    await applyZoneCommand({ command: "set_mute", muted: false }, controls);
    await applyZoneCommand({ command: "set_mute", muted: true }, controls);
    await applyZoneCommand({ command: "jump", queue_position: 3 }, controls);
    await applyZoneCommand(
      { command: "load", song_ids: ["a", "b"], current_position: 1 },
      controls,
    );
    expect(calls).toEqual([
      "seek:42.5",
      "volume:100",
      "muted:true",
      "jump:3",
      "load:a,b@1+0:true",
    ]);
  });
});

describe("local zone reporting", () => {
  test("status and heartbeat cadence", () => {
    expect(
      localZoneStatus({
        songId: "",
        queueSongIds: [],
        queueIndex: 0,
        isPlaying: false,
        hasError: false,
      }),
    ).toBe("idle");
    expect(
      localZoneStatus({
        songId: "a",
        queueSongIds: ["a"],
        queueIndex: 0,
        isPlaying: true,
        hasError: false,
      }),
    ).toBe("playing");
    expect(heartbeatInterval("playing", false)).toBeLessThan(
      heartbeatInterval("paused", false),
    );
  });

  test("queue keys change with order and content", () => {
    expect(queueKey(["a", "b"])).toBe(queueKey(["a", "b"]));
    expect(queueKey(["a", "b"])).not.toBe(queueKey(["b", "a"]));
    expect(queueKey(["ab"])).not.toBe(queueKey(["a", "b"]));
  });

  test("device identity is stable and readable", () => {
    const store = new Map<string, string>();
    const storage = {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => void store.set(key, value),
    };
    const first = readOrCreateDeviceId(storage, () => "device12345678");
    expect(readOrCreateDeviceId(storage, () => "other12345678")).toBe(first);
    expect(readOrCreateDeviceId(null, () => "fallback1234")).toBe(
      "fallback1234",
    );
    const chromeMac =
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36";
    expect(describeBrowser(chromeMac)).toBe("Chrome on macOS");
    expect(
      describeBrowser(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:130.0) Gecko/20100101 Firefox/130.0",
      ),
    ).toBe("Firefox on Windows");
    expect(clientZoneIdentity(chromeMac, true)).toEqual({
      kind: "desktop",
      name: "Mujin Music on macOS",
      device: "Desktop app",
    });
    expect(clientZoneIdentity(chromeMac, false).kind).toBe("web");
  });
});

describe("history zone", () => {
  test("plays are attributed to this device's zone", () => {
    const chromeMac =
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";
    expect(localZoneForHistory("device-1234", chromeMac, true)).toEqual({
      zone_id: "client:device-1234",
      zone_name: "Mujin Music on macOS",
      device: "Desktop app",
    });
    const web = localZoneForHistory("bad id!", chromeMac, false);
    expect(web.zone_id).toBeUndefined();
    expect(web.device).toBe(web.zone_name);
  });
});
