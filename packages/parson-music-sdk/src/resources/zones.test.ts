import { afterEach, expect, mock, spyOn, test } from "bun:test";
import { formatSampleRate } from "../player/signal-path";
import api from "../core/http";
import {
  bridgeBackendLabel,
  zoneOutputsSummary,
  addBridge,
  getBridges,
  removeBridge,
  createZoneDeviceId,
  estimateZonePosition,
  formatSleepRemaining,
  getZoneEventsURL,
  getZones,
  groupingPlan,
  levelingGainDb,
  resolveLevelBy,
  setZoneViewerDevice,
  updateZoneSettings,
  zoneIconName,
  zoneSettingsDraft,
  zoneSettingsPatch,
  zoneSettingsProblem,
  zoneStatusLabel,
  zoneSliderMax,
  isEditableQueueItem,
  nextRepeatMode,
  orderZones,
  previewQueueEdit,
  playOnZoneCommand,
  queueOnZoneCommand,
  MAX_ZONE_QUEUE_SONGS,
  sleepTimerLabel,
  reportClientZone,
  sendZoneCommand,
  transferZone,
  ungroupZones,
  unseenZoneCommands,
  anyZonePlaying,
  deviceVolumeLabel,
  discoverOutputs,
  dspSummary,
  pauseAllZones,
  zoneKindLabel,
  streamQuality,
  streamQualityText,
  zoneQualityLabel,
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

const settingsView = (
  overrides: Partial<NonNullable<Zone["settings"]>> = {},
): NonNullable<Zone["settings"]> => ({
  default_name: "Living Room",
  custom_name: true,
  icon: null,
  crossfade_seconds: 0,
  leveling: "off",
  leveling_target_lufs: -18,
  leveling_adjustment_db: 0,
  private: false,
  supports: {
    crossfade: true,
    leveling: true,
    volume_limits: true,
    private: false,
  },
  notes: [],
  ...overrides,
});

test("saving zone settings leaves an unchanged name out of the request", () => {
  const lounge = zone({
    id: "sonos:RINCON_1",
    kind: "sonos",
    name: "Lounge",
    volume: {
      level: 30,
      muted: false,
      min: 0,
      max: 100,
      comfort: null,
      step: 2,
    },
    settings: settingsView(),
  });
  const initial = zoneSettingsDraft(lounge);
  // Only the crossfade changed: no name, and certainly no `name: null`.
  const patch = zoneSettingsPatch(initial, {
    ...initial,
    crossfade_seconds: 4,
  });
  expect(patch).toEqual({ crossfade_seconds: 4 });
  expect("name" in patch).toBe(false);
  // Whitespace-only edits are not a rename.
  expect(zoneSettingsPatch(initial, { ...initial, name: " Lounge " })).toEqual(
    {},
  );
  // A new name is sent; clearing the field resets to the device's name.
  expect(zoneSettingsPatch(initial, { ...initial, name: "Den" })).toEqual({
    name: "Den",
  });
  expect(zoneSettingsPatch(initial, { ...initial, name: "  " })).toEqual({
    reset_name: true,
  });
  // Limits and leveling are sent as changed, `null` clears a limit.
  const limited = { ...initial, comfort_volume: 40, safety_max_volume: 70 };
  expect(zoneSettingsPatch(initial, limited)).toEqual({
    comfort_volume: 40,
    safety_max_volume: 70,
  });
  expect(
    zoneSettingsPatch(limited, { ...limited, comfort_volume: null }),
  ).toEqual({ comfort_volume: null });
  expect(
    zoneSettingsPatch(initial, {
      ...initial,
      leveling: "album",
      leveling_target_lufs: -14,
      icon: "sofa",
      private: true,
    }),
  ).toEqual({
    leveling: "album",
    leveling_target_lufs: -14,
    icon: "sofa",
    private: true,
  });
});

test("zone settings drafts read limits from the zone volume", () => {
  const draft = zoneSettingsDraft(
    zone({
      volume: {
        level: 30,
        muted: false,
        min: 10,
        max: 70,
        comfort: 40,
        step: 2,
      },
      settings: settingsView({ icon: "sofa", leveling: "auto" }),
    }),
  );
  expect(draft).toMatchObject({
    comfort_volume: 40,
    safety_min_volume: 10,
    safety_max_volume: 70,
    icon: "sofa",
    leveling: "auto",
  });
  expect(zoneSettingsProblem(draft)).toBeNull();
  expect(zoneSettingsProblem({ ...draft, safety_min_volume: 80 })).toContain(
    "below",
  );
  expect(zoneSettingsProblem({ ...draft, comfort_volume: 90 })).toContain(
    "inside",
  );
  // An older zone without settings still gives a usable draft.
  expect(zoneSettingsDraft(zone({})).leveling).toBe("off");
});

test("volume sliders stop at the comfort limit", () => {
  const volume = {
    level: 30,
    muted: false,
    min: 0,
    max: 70,
    comfort: 40,
    step: 2,
  };
  expect(zoneSliderMax(volume)).toBe(40);
  expect(zoneSliderMax({ ...volume, comfort: null })).toBe(70);
  expect(zoneSliderMax({ ...volume, comfort: undefined })).toBe(70);
});

test("zone icons fall back to the kind's icon", () => {
  expect(zoneIconName({ kind: "sonos" })).toBe("speaker");
  expect(zoneIconName({ kind: "sonos", icon: "sofa" })).toBe("sofa");
  expect(zoneIconName({ kind: "mobile", icon: "rocket" })).toBe("phone");
});

test("leveling gain matches the server's", () => {
  // Same vectors as the Rust tests in zones/leveling.rs.
  const loud = { integrated_lufs: -8, true_peak_db: 0.5 };
  expect(levelingGainDb(loud, -18, 0)).toBe(-10);
  expect(levelingGainDb(loud, -18, 3)).toBe(-7);
  expect(
    levelingGainDb({ integrated_lufs: -28, true_peak_db: -6 }, -18, 0),
  ).toBe(5);
  expect(
    levelingGainDb({ integrated_lufs: -70, true_peak_db: -60 }, -18, 0),
  ).toBe(12);
});

test("auto leveling uses the album only while an album plays in order", () => {
  const queue = ["a1", "a2", "b1", "c1"];
  const albumOf = (id: string) => id.slice(0, 1);
  expect(resolveLevelBy("auto", queue, 0, albumOf)).toBe("album");
  expect(resolveLevelBy("auto", queue, 1, albumOf)).toBe("album");
  expect(resolveLevelBy("auto", queue, 2, albumOf)).toBe("track");
  expect(resolveLevelBy("off", queue, 0, albumOf)).toBeNull();
  expect(resolveLevelBy("album", queue, 3, albumOf)).toBe("album");
});

test("zone requests name this device so its private zone stays visible", async () => {
  const get = spyOn(api, "get").mockResolvedValue(
    ok({ zones: [], server_time_ms: 0, sonos: null }),
  );
  const patch = spyOn(api, "patch").mockResolvedValue(ok({ zone: null }));
  setZoneViewerDevice("desktop-private1");
  try {
    await getZones();
    expect(get).toHaveBeenCalledWith("/zones", {
      timeout: 5_000,
      headers: { "X-Parson-Zone-Device": "desktop-private1" },
    });
    await updateZoneSettings("client:desktop-private1", { private: true });
    expect(patch).toHaveBeenCalledWith(
      "/zones/client%3Adesktop-private1/settings",
      { private: true },
      { headers: { "X-Parson-Zone-Device": "desktop-private1" } },
    );
  } finally {
    setZoneViewerDevice(null);
  }
  await getZones();
  expect(get).toHaveBeenLastCalledWith("/zones", { timeout: 5_000 });
});

test("audio devices and UPnP renderers sort, label and describe their output", () => {
  const dac = zone({
    id: "device:alsa-e30-0",
    kind: "device",
    ecosystem: "device",
    name: "Topping E30",
    status: "playing",
    volume: {
      level: 80,
      muted: false,
      min: 0,
      max: 100,
      comfort: null,
      step: 2,
      device_range: {
        min: -127.5,
        max: 0,
        step: 0.5,
        unit: "dB",
        hardware: true,
        current: -16,
      },
    },
    output: {
      max_sample_rate: 384_000,
      max_bit_depth: 32,
      formats: ["FLAC"],
      note: "",
      exclusive: true,
      volume_mode: "hardware",
      stream: {
        source: "FLAC 96 kHz/24-bit",
        output: "96 kHz/24-bit (S24_3LE) to Topping E30, exclusive",
        bit_perfect: true,
        steps: [],
      },
    },
  });
  const renderer = zone({
    id: "upnp:abc",
    kind: "upnp",
    ecosystem: "upnp",
    name: "Streamer",
    volume: {
      level: 50,
      muted: false,
      min: 0,
      max: 100,
      step: 2,
      device_range: {
        min: 0,
        max: 30,
        step: 1,
        unit: "steps",
        hardware: true,
        current: 15,
      },
    },
  });
  const sonos = zone({ id: "sonos:R", kind: "sonos", name: "Kitchen" });
  const browser = zone({ id: "client:b", kind: "web", name: "Browser" });
  expect(
    orderZones([renderer, sonos, dac, browser]).map((item) => item.id),
  ).toEqual(["client:b", "device:alsa-e30-0", "sonos:R", "upnp:abc"]);
  expect(zoneKindLabel("upnp")).toBe("UPnP/DLNA renderer");
  expect(zoneKindLabel("device")).toBe("Audio device");
  expect(zoneIconName(dac)).toBe("amplifier");
  expect(zoneIconName(renderer)).toBe("radio");
  expect(deviceVolumeLabel(dac.volume)).toBe(
    "-16.0 dB on the device (-127.5 to 0.0 dB)",
  );
  expect(deviceVolumeLabel(renderer.volume)).toBe("15 of 30 on the device");
  expect(deviceVolumeLabel(browser.volume)).toBeNull();
  expect(zoneQualityLabel(dac)).toBe("Bit-perfect");
  expect(zoneQualityLabel(renderer)).toBeNull();
  // One rate label for zones and the signal path.
  expect(formatSampleRate(44_100)).toBe("44.1kHz");
  expect(formatSampleRate(352_800)).toBe("352.8kHz");
  expect(formatSampleRate(96_000)).toBe("96kHz");
  expect(anyZonePlaying([dac, renderer])).toBe(true);
  expect(anyZonePlaying([renderer, { ...dac, grouped_into: "linked:x" }])).toBe(
    false,
  );
});

test("DSP summaries match the server and only changed DSP is saved", () => {
  expect(dspSummary(undefined)).toBe("Off");
  expect(
    dspSummary({
      enabled: true,
      preamp_db: 0,
      bass_db: 3,
      treble_db: -2,
      crossfeed: true,
    }),
  ).toBe("Preamp \u22123 dB, Bass +3 dB, Treble \u22122 dB, Crossfeed");
  expect(
    dspSummary({
      enabled: false,
      preamp_db: -6,
      bass_db: 0,
      treble_db: 0,
      crossfeed: false,
    }),
  ).toBe("Off");
  const dac = zone({
    id: "device:dac",
    kind: "device",
    settings: settingsView({
      dsp: {
        enabled: false,
        preamp_db: 0,
        bass_db: 0,
        treble_db: 0,
        crossfeed: false,
      },
      exclusive: true,
      volume_mode: "hardware",
      max_sample_rate: null,
      max_bit_depth: null,
    }),
  });
  const initial = zoneSettingsDraft(dac);
  expect(zoneSettingsPatch(initial, { ...initial })).toEqual({});
  expect(
    zoneSettingsPatch(initial, {
      ...initial,
      dsp: { ...initial.dsp, enabled: true, bass_db: 4 },
      exclusive: false,
      volume_mode: "fixed",
      max_sample_rate: 96_000,
    }),
  ).toEqual({
    dsp: {
      enabled: true,
      preamp_db: 0,
      bass_db: 4,
      treble_db: 0,
      crossfeed: false,
    },
    exclusive: false,
    volume_mode: "fixed",
    max_sample_rate: 96_000,
  });
});

test("Home Assistant volume control and standby read and save", () => {
  const amp = zone({
    id: "device:dac",
    kind: "device",
    status: "standby",
    source: "parson",
    settings: settingsView({ volume_control: "home_assistant" }),
  });
  expect(zoneStatusLabel(amp)).toBe("Standby");
  const initial = zoneSettingsDraft(amp);
  expect(initial.volume_control).toBe("home_assistant");
  expect(
    zoneSettingsPatch(initial, { ...initial, volume_control: "parson" }),
  ).toEqual({ volume_control: "parson" });
  expect(zoneSettingsDraft(zone({})).volume_control).toBe("parson");
});

test("pause all and output discovery call the zone API", async () => {
  const post = spyOn(api, "post").mockResolvedValue(ok({ paused: 3 }));
  expect(await pauseAllZones()).toEqual({ paused: 3 });
  expect(post).toHaveBeenCalledWith(
    "/zones/pause-all",
    {},
    { timeout: 15_000 },
  );
  await discoverOutputs();
  expect(post).toHaveBeenLastCalledWith(
    "/zones/outputs/discover",
    {},
    { timeout: 20_000 },
  );
});

test("signal path ratings read like Roon's quality light", () => {
  const playing = (stream: NonNullable<Zone["output"]>["stream"]) =>
    zone({
      kind: "sonos",
      output: {
        max_sample_rate: 48_000,
        max_bit_depth: 24,
        formats: ["FLAC"],
        note: "",
        stream,
      },
    });
  const sonos = playing({
    source: "FLAC 96 kHz/24-bit",
    output: "FLAC 48 kHz/24-bit to Living Room (Sonos Five)",
    bit_perfect: false,
    steps: ["Sample rate conversion 96 kHz to 48 kHz (linear phase)"],
    quality: "high",
  });
  expect(zoneQualityLabel(sonos)).toBe("High quality");
  const ogg = playing({
    source: "Ogg Vorbis 44.1 kHz",
    output: "FLAC 44.1 kHz/24-bit to Den (Sonos One)",
    bit_perfect: false,
    steps: ["Decoded Ogg Vorbis to 24-bit PCM"],
    quality: "low",
  });
  expect(zoneQualityLabel(ogg)).toBe("Low quality");
  expect(streamQualityText("enhanced").label).toBe("Enhanced");
  // Servers from before the rating still get a sensible one.
  expect(
    streamQuality({ source: "", output: "", bit_perfect: true, steps: [] }),
  ).toBe("lossless");
  expect(
    streamQuality({ source: "", output: "", bit_perfect: false, steps: [] }),
  ).toBe("high");
});

test("bridge zones group sample-synced and bridges are managed by address", async () => {
  const kitchen = zone({
    id: "bridge:pi:hw-2-0",
    kind: "bridge",
    ecosystem: "bridge",
    name: "Kitchen DAC",
  });
  const office = zone({
    id: "bridge:mini:hw-1-0",
    kind: "bridge",
    ecosystem: "bridge",
    name: "Office DAC",
  });
  const sonos = zone({ id: "sonos:A", kind: "sonos", ecosystem: "sonos" });
  const plan = groupingPlan([kitchen, office]);
  expect(plan).toMatchObject({ allowed: true, synced: true });
  expect(plan.reason).toContain("Kitchen DAC's DAC clock");
  expect(groupingPlan([kitchen, sonos])).toMatchObject({ synced: false });
  const group = zone({
    id: "bridge-group:1",
    kind: "bridge",
    ecosystem: "bridge",
    group: { id: "bridge-group:1", synced: true, sync_note: "", members: [] },
  });
  expect(groupingPlan([group, kitchen]).allowed).toBeFalse();
  expect(zoneKindLabel("bridge")).toBe("Parson Bridge");
  expect(zoneKindLabel("airplay")).toBe("AirPlay");
  expect(zoneIconName({ kind: "airplay" })).toBe("speaker");
  expect(
    deviceVolumeLabel({
      level: 30,
      muted: false,
      min: 0,
      max: 100,
      step: 1,
      device_range: {
        min: 0,
        max: 1,
        step: 0.01,
        unit: "level",
        hardware: true,
        current: 0.3,
      },
    }),
  ).toBe("30% on the device");

  const get = spyOn(api, "get").mockResolvedValue({
    data: { enabled: true, bridges: [] },
  } as never);
  const post = spyOn(api, "post").mockResolvedValue({
    data: { address: "pi.local:1994", connected: true },
  } as never);
  const remove = spyOn(api, "delete").mockResolvedValue({
    data: null,
  } as never);
  expect((await getBridges()).enabled).toBeTrue();
  expect(get.mock.calls[0]![0]).toBe("/zones/bridges");
  expect((await addBridge("pi.local")).connected).toBeTrue();
  expect(post).toHaveBeenCalledWith(
    "/zones/bridges",
    { address: "pi.local" },
    { timeout: 20_000 },
  );
  expect(zoneOutputsSummary(null)).toBeNull();
  expect(
    zoneOutputsSummary({
      upnp: { enabled: false, renderers: 0, last_error: null },
      devices: { enabled: false, backend: null, devices: 0, note: null },
      bridges: {
        enabled: true,
        bridges: [
          {
            address: "a",
            source: "discovered",
            connected: true,
            bridge_id: "a",
            name: "pi",
            host: "pi",
            platform: null,
            version: null,
            backend: "ALSA",
            devices: 2,
            clock_offset_us: 0,
            clock_uncertainty_us: 100,
            note: null,
            last_error: null,
          },
        ],
      },
      chromecast: { enabled: true, note: null, devices: [] },
    }),
  ).toBe("1 Parson Bridge (2 outputs)");
  await removeBridge("pi.local:1994");
  expect(remove).toHaveBeenCalledWith("/zones/bridges/pi.local%3A1994");
});

test("library actions become zone commands within the queue limit", () => {
  expect(playOnZoneCommand(["a", "b", "c"], 1)).toEqual({
    command: "load",
    song_ids: ["a", "b", "c"],
    current_position: 1,
    position_ms: 0,
    play: true,
  });
  // A huge playlist keeps the chosen track, centred in what is sent.
  const many = Array.from({ length: 1_200 }, (_, index) => `s${index}`);
  const load = playOnZoneCommand(many, 900);
  expect(load.song_ids).toHaveLength(MAX_ZONE_QUEUE_SONGS);
  expect(load.song_ids[load.current_position!]).toBe("s900");
  expect(playOnZoneCommand(many, 1_199).song_ids.at(-1)).toBe("s1199");
  expect(() => playOnZoneCommand([], 0)).toThrow();
  expect(queueOnZoneCommand(["x"], true)).toEqual({
    command: "add_to_queue",
    song_ids: ["x"],
    next: true,
  });
  expect(
    previewQueueEdit(["a", "b", "c"], 0, queueOnZoneCommand(["x", "y"], true)),
  ).toEqual(["a", "x", "y", "b", "c"]);
  expect(
    previewQueueEdit(["a", "b"], 0, queueOnZoneCommand(["z"], false)),
  ).toEqual(["a", "b", "z"]);
  expect(previewQueueEdit([], 0, queueOnZoneCommand(["z"], true))).toEqual([
    "z",
  ]);
});

test("bridge audio APIs read as the mode they play in", () => {
  expect(bridgeBackendLabel("WASAPI")).toBe("WASAPI exclusive");
  expect(bridgeBackendLabel("CoreAudio")).toBe("CoreAudio hog mode");
  expect(bridgeBackendLabel("ALSA")).toBe("ALSA");
  expect(bridgeBackendLabel(null)).toBeNull();
});
