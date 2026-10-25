import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api from "../core/http";
import {
  createZoneDeviceId,
  estimateZonePosition,
  formatSleepRemaining,
  getZoneEventsURL,
  groupingPlan,
  isEditableQueueItem,
  nextRepeatMode,
  orderZones,
  previewQueueEdit,
  sleepTimerLabel,
  reportClientZone,
  sendZoneCommand,
  transferZone,
  ungroupZones,
  unseenZoneCommands,
  type Zone,
} from "./zones";

afterEach(() => mock.restore());

const zone = (overrides: Partial<Zone>): Zone => ({
  id: "client:a",
  kind: "web",
  ecosystem: "parson",
  name: "Zone",
  device: "",
  status: "idle",
  source: "none",
  now_playing: null,
  queue: { song_ids: [], current_position: 0 },
  repeat: "off",
  shuffle: false,
  position_ms: 0,
  duration_ms: 0,
  volume: null,
  group: null,
  grouped_into: null,
  capabilities: {
    transport: true,
    seek: true,
    volume: true,
    mute: true,
    queue: true,
    queue_edit: true,
    repeat: true,
    shuffle: true,
    group: true,
    transfer_target: true,
    sleep_timer: true,
  },
  sleep_timer: null,
  notes: [],
  ...overrides,
});

const ok = (data: unknown = {}) =>
  ({ data, status: 200, headers: new Headers() }) as never;

test("zone events use the API WebSocket origin", () => {
  expect(getZoneEventsURL()).toBe("ws://localhost:1993/api/v1/zones/events");
});

test("zone ids are path-encoded for commands and groups", async () => {
  const post = spyOn(api, "post").mockResolvedValue(ok());
  const remove = spyOn(api, "delete").mockResolvedValue(ok());
  await sendZoneCommand("sonos:RINCON_1/x", {
    command: "set_volume",
    volume: 20,
    member_id: "sonos:RINCON_2",
  });
  expect(post).toHaveBeenCalledWith(
    "/zones/sonos%3ARINCON_1%2Fx/commands",
    { command: "set_volume", volume: 20, member_id: "sonos:RINCON_2" },
    { timeout: 8_000 },
  );
  await ungroupZones("linked:abc", "client:a");
  expect(remove).toHaveBeenCalledWith("/zones/groups/linked%3Aabc", {
    params: { member: "client:a" },
    timeout: 15_000,
  });
});

test("transfers and client reports hit the backend routes", async () => {
  const post = spyOn(api, "post").mockResolvedValue(ok({ zone: null }));
  const put = spyOn(api, "put").mockResolvedValue(
    ok({ zone_id: "client:dev", commands: [], needs_queue: false }),
  );
  await transferZone("client:dev", "sonos:RINCON_1");
  expect(post).toHaveBeenCalledWith(
    "/zones/transfer",
    { from: "client:dev", to: "sonos:RINCON_1" },
    { timeout: 30_000 },
  );
  const songIds = Array.from({ length: 600 }, (_, index) => `s${index}`);
  await reportClientZone("dev", {
    kind: "web",
    name: "Chrome on Linux",
    status: "playing",
    song_ids: songIds,
    current_position: 0,
    position_ms: 0,
    duration_ms: 0,
    volume: 50,
    muted: false,
    acknowledged_seq: 0,
  });
  const body = put.mock.calls[0]![1] as { song_ids: string[] };
  expect(put.mock.calls[0]![0]).toBe("/zones/clients/dev");
  expect(body.song_ids).toHaveLength(500);
});

test("grouping plans are explicit about sync", () => {
  const sonosA = zone({ id: "sonos:A", kind: "sonos", ecosystem: "sonos" });
  const sonosB = zone({ id: "sonos:B", kind: "sonos", ecosystem: "sonos" });
  const browser = zone({ id: "client:x" });
  const cast = zone({ id: "cast:1", kind: "chromecast" });
  expect(groupingPlan([sonosA]).allowed).toBeFalse();
  expect(groupingPlan([sonosA, sonosB])).toMatchObject({
    allowed: true,
    synced: true,
  });
  const linked = groupingPlan([sonosA, browser]);
  expect(linked).toMatchObject({ allowed: true, synced: false });
  expect(linked.reason).toContain("not sample-synced");
  expect(groupingPlan([browser, cast]).reason).toContain("Google Home");
});

test("zones order with this device first, then apps, groups and Sonos", () => {
  const ordered = orderZones(
    [
      zone({ id: "sonos:K", kind: "sonos", name: "Kitchen" }),
      zone({ id: "client:b", kind: "mobile", name: "Phone" }),
      zone({ id: "linked:1", kind: "linked", name: "Phone + Kitchen" }),
      zone({ id: "client:a", kind: "web", name: "This" }),
    ],
    "client:a",
  );
  expect(ordered.map((entry) => entry.id)).toEqual([
    "client:a",
    "client:b",
    "linked:1",
    "sonos:K",
  ]);
});

test("positions extrapolate while playing and commands apply once", () => {
  expect(
    estimateZonePosition(
      { position_ms: 1_000, duration_ms: 5_000, status: "playing" },
      0,
      2_000,
    ),
  ).toBe(3_000);
  expect(
    estimateZonePosition(
      { position_ms: 1_000, duration_ms: 5_000, status: "paused" },
      0,
      2_000,
    ),
  ).toBe(1_000);
  expect(
    estimateZonePosition(
      { position_ms: 4_000, duration_ms: 5_000, status: "playing" },
      0,
      9_000,
    ),
  ).toBe(5_000);
  const commands = unseenZoneCommands(
    [
      { seq: 3, command: { command: "play" } },
      { seq: 2, command: { command: "pause" } },
      { seq: 3, command: { command: "play" } },
      { seq: 1, command: { command: "next" } },
    ],
    1,
  );
  expect(commands.map((command) => command.seq)).toEqual([2, 3]);
});

test("device ids are server-valid and sleep labels are readable", () => {
  const id = createZoneDeviceId();
  expect(id).toMatch(/^[a-z0-9]{20}$/);
  expect(formatSleepRemaining(90 * 60_000, 0)).toBe("1 h 30 min");
  expect(formatSleepRemaining(60 * 60_000, 0)).toBe("1 h");
  expect(formatSleepRemaining(29_000, 0)).toBe("1 min");
});

test("queue edits preview like the server: upcoming items only, guarded", () => {
  const ids = ["a", "b", "c", "d"];
  expect(
    previewQueueEdit(ids, 1, {
      command: "move_queue_item",
      from: 3,
      to: 2,
      song_id: "d",
    }),
  ).toEqual(["a", "b", "d", "c"]);
  expect(
    previewQueueEdit(ids, 1, { command: "move_queue_item", from: 1, to: 3 }),
  ).toBeNull();
  expect(
    previewQueueEdit(ids, 1, {
      command: "remove_queue_item",
      queue_position: 2,
      song_id: "x",
    }),
  ).toBeNull();
  expect(
    previewQueueEdit(ids, 1, {
      command: "remove_queue_item",
      queue_position: 2,
    }),
  ).toEqual(["a", "b", "d"]);
  expect(previewQueueEdit(ids, 1, { command: "clear_upcoming" })).toEqual([
    "a",
    "b",
  ]);
  const remote = zone({
    queue: { song_ids: ids, current_position: 1 },
  });
  expect(isEditableQueueItem(remote, 1)).toBe(false);
  expect(isEditableQueueItem(remote, 2)).toBe(true);
  expect(
    isEditableQueueItem(
      {
        ...remote,
        capabilities: { ...remote.capabilities, queue_edit: false },
      },
      2,
    ),
  ).toBe(false);
  expect(nextRepeatMode("off")).toBe("all");
  expect(nextRepeatMode("all")).toBe("one");
  expect(nextRepeatMode("one")).toBe("off");
});

test("queue commands post through the zone command API", async () => {
  const post = spyOn(api, "post").mockResolvedValue({ data: {} } as never);
  await sendZoneCommand("sonos:RINCON_1", {
    command: "remove_queue_item",
    queue_position: 4,
    song_id: "song-9",
  });
  expect(post).toHaveBeenCalledWith(
    "/zones/sonos%3ARINCON_1/commands",
    { command: "remove_queue_item", queue_position: 4, song_id: "song-9" },
    { timeout: 8_000 },
  );
});

test("sleep timer labels say what will happen", () => {
  const timer = {
    ends_at_ms: 10 * 60_000,
    finishing_track: false,
    fading: false,
  };
  expect(sleepTimerLabel(timer, 0)).toBe("Sleep in 10 min");
  expect(sleepTimerLabel({ ...timer, finishing_track: true }, 0)).toBe(
    "Sleep after this track",
  );
  expect(sleepTimerLabel({ ...timer, fading: true }, 0)).toBe(
    "Fading out to sleep",
  );
});
