import { describe, expect, test } from "bun:test";
import type { NativeOutputSession } from "@parson/music-sdk";
import {
  NativeOutputElement,
  songIdFromStreamUrl,
  type NativeOutputTransport,
} from "./native-output-element";

function session(
  patch: Partial<NativeOutputSession> = {},
): NativeOutputSession {
  return {
    device_id: "alsa:USB:0",
    device_name: "USB DAC",
    song_id: "song 1",
    user_id: 1,
    duration_ms: 200_000,
    state: "playing",
    position_ms: 0,
    error: null,
    plan: {} as NativeOutputSession["plan"],
    signal_path: {} as NativeOutputSession["signal_path"],
    ...patch,
  };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function fakeTransport() {
  const calls: string[] = [];
  let current: NativeOutputSession | null = null;
  const transport: NativeOutputTransport = {
    play: async (input) => {
      calls.push(
        `play ${input.song_id} @${input.position_ms}${input.leveling ? ` ${input.leveling.gain_db}dB` : ""}`,
      );
      current = session({
        song_id: input.song_id,
        position_ms: input.position_ms,
      });
      return current;
    },
    command: async (command, position) => {
      calls.push(position === undefined ? command : `${command} ${position}`);
      if (!current) return null;
      if (command === "stop") current = null;
      else if (command === "seek")
        current = { ...current, position_ms: position ?? 0 };
      else if (command === "pause") current = { ...current, state: "paused" };
      else if (command === "resume") {
        // The server answers before acting on the command.
        current = { ...current, state: "playing" };
        return { ...current, state: "paused" };
      }
      return current;
    },
    session: async () => current,
  };
  return {
    calls,
    transport,
    set: (next: NativeOutputSession | null) => {
      current = next;
    },
  };
}

function events(element: EventTarget) {
  const seen: string[] = [];
  for (const name of [
    "play",
    "playing",
    "pause",
    "ended",
    "error",
    "loadedmetadata",
  ])
    element.addEventListener(name, () => seen.push(name));
  return seen;
}

describe("native output element", () => {
  test("stream URLs map to song ids", () => {
    expect(
      songIdFromStreamUrl(
        "http://host/api/v1/media/songs/song%201/stream?bitrate=0",
      ),
    ).toBe("song 1");
    expect(songIdFromStreamUrl("http://host/media/songs/abc/stream")).toBe(
      "abc",
    );
    expect(songIdFromStreamUrl("http://host/other")).toBeNull();
  });

  test("plays from the chosen position and mirrors the session", async () => {
    let clock = 0;
    const fake = fakeTransport();
    const sessions: (NativeOutputSession | null)[] = [];
    const element = new NativeOutputElement("alsa:USB:0", {
      transport: fake.transport,
      pollMs: 60_000,
      now: () => clock,
      onSession: (next) => sessions.push(next),
    });
    const seen = events(element);
    element.src = "http://h/media/songs/song%201/stream";
    element.currentTime = 12.5;
    await element.play();
    expect(fake.calls).toEqual(["play song 1 @12500"]);
    expect(element.paused).toBeFalse();
    expect(element.duration).toBe(200);
    clock = 1_000;
    expect(element.currentTime).toBeCloseTo(13.5, 3);
    expect(seen).toEqual(["loadedmetadata", "play", "playing"]);
    expect(sessions.at(-1)?.device_id).toBe("alsa:USB:0");

    element.pause();
    expect(element.paused).toBeTrue();
    clock = 5_000;
    expect(element.currentTime).toBeCloseTo(13.5, 3);
    // Resuming does not reopen the track or bounce back to paused.
    await element.play();
    expect(fake.calls.slice(1)).toEqual(["pause", "resume"]);
    expect(element.paused).toBeFalse();
    expect(seen.filter((name) => name === "pause")).toHaveLength(1);

    element.currentTime = 60;
    await flush();
    expect(fake.calls.at(-1)).toBe("seek 60000");

    // Turning leveling on restarts the track on the server with the gain.
    element.leveling = { gain_db: -6.4, source: "track" };
    await flush();
    expect(fake.calls.at(-1)).toBe("play song 1 @60000 -6.4dB");
    const count = fake.calls.length;
    element.leveling = { gain_db: -6.41, source: "track" };
    expect(fake.calls).toHaveLength(count);
  });

  test("changing track pauses the old one before the new one starts", async () => {
    const fake = fakeTransport();
    const order: string[] = [];
    let release: () => void = () => {};
    const slowPause = new Promise<void>((resolve) => {
      release = resolve;
    });
    const element = new NativeOutputElement("alsa:USB:0", {
      transport: {
        ...fake.transport,
        command: async (command, position) => {
          if (command === "pause") await slowPause;
          order.push(command);
          return fake.transport.command(command, position);
        },
        play: async (input) => {
          order.push(`play ${input.song_id}`);
          return fake.transport.play(input);
        },
      },
      pollMs: 60_000,
    });
    element.src = "http://h/media/songs/a/stream";
    await element.play();
    element.src = "http://h/media/songs/b/stream";
    const started = element.play();
    await flush();
    release();
    await started;
    expect(order).toEqual(["play a", "pause", "play b"]);
    expect(element.paused).toBeFalse();
  });

  test("ends, fails and notices when the device is taken away", async () => {
    const fake = fakeTransport();
    const element = new NativeOutputElement("alsa:USB:0", {
      transport: fake.transport,
      pollMs: 60_000,
    });
    const seen = events(element);
    element.src = "http://h/media/songs/a/stream";
    await element.play();
    fake.set(session({ song_id: "a", state: "ended", position_ms: 199_990 }));
    await element.poll();
    expect(element.ended).toBeTrue();
    expect(element.paused).toBeTrue();
    expect(element.currentTime).toBe(200);
    expect(seen.filter((name) => name === "ended")).toHaveLength(1);
    await element.poll();
    expect(seen.filter((name) => name === "ended")).toHaveLength(1);

    // Playing after the end starts the track over.
    await element.play();
    expect(fake.calls.at(-1)).toBe("play a @0");
    fake.set(session({ song_id: "a", state: "failed", error: "busy" }));
    await element.poll();
    expect(element.error?.message).toBe("busy");
    expect(seen.at(-1)).toBe("error");

    await element.play();
    fake.set(session({ song_id: "other" }));
    await element.poll();
    expect(element.paused).toBeTrue();
    expect(seen.at(-1)).toBe("pause");
  });

  test("a failed start rejects like a media element and dispose stops the device", async () => {
    const fake = fakeTransport();
    const failing = new NativeOutputElement("alsa:USB:0", {
      transport: {
        ...fake.transport,
        play: async () => {
          throw new Error("The device is in use by another program");
        },
      },
    });
    failing.src = "http://h/media/songs/a/stream";
    await expect(failing.play()).rejects.toThrow("in use");
    expect(failing.error?.code).toBe(4);

    const element = new NativeOutputElement("alsa:USB:0", {
      transport: fake.transport,
      pollMs: 60_000,
    });
    element.src = "http://h/media/songs/a/stream";
    await element.play();
    element.dispose();
    await flush();
    expect(fake.calls.at(-1)).toBe("stop");
    await expect(element.play()).rejects.toThrow();
  });
});
