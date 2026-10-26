import { expect, test } from "bun:test";
import { CROSSFEED_LEVEL, createZoneDspChain, dspParameters } from "./zone-dsp";

test("DSP parameters are neutral when off and keep headroom when on", () => {
  expect(dspParameters(null)).toEqual({
    active: false,
    preampGain: 1,
    bassDb: 0,
    trebleDb: 0,
    crossfeed: false,
  });
  expect(
    dspParameters({
      enabled: false,
      preamp_db: -6,
      bass_db: 4,
      treble_db: 0,
      crossfeed: true,
    }).active,
  ).toBe(false);
  const boosted = dspParameters({
    enabled: true,
    preamp_db: 0,
    bass_db: 6,
    treble_db: 2,
    crossfeed: true,
  });
  expect(boosted.active).toBe(true);
  // The largest boost (6 dB) comes off the level so the EQ cannot clip.
  expect(boosted.preampGain).toBeCloseTo(10 ** (-6 / 20), 6);
  expect(boosted.bassDb).toBe(6);
  expect(boosted.crossfeed).toBe(true);
});

type FakeParam = { value: number; target?: number };
function param(value = 0): FakeParam & {
  setTargetAtTime: (target: number) => void;
} {
  const result: FakeParam & { setTargetAtTime: (target: number) => void } = {
    value,
    setTargetAtTime(target: number) {
      result.target = target;
    },
  };
  return result;
}

test("the browser DSP chain sets shelves, preamp and crossfeed", () => {
  const nodes: Record<string, unknown>[] = [];
  const node = (extra: Record<string, unknown>) => {
    const created = {
      connections: [] as unknown[],
      connect(target: unknown) {
        created.connections.push(target);
        return target;
      },
      ...extra,
    };
    nodes.push(created);
    return created;
  };
  const context = {
    currentTime: 0,
    createGain: () => node({ kind: "gain", gain: param(1) }),
    createBiquadFilter: () =>
      node({
        kind: "biquad",
        type: "",
        frequency: param(),
        gain: param(),
      }),
    createChannelSplitter: () => node({ kind: "splitter" }),
    createChannelMerger: () => node({ kind: "merger" }),
    createDelay: () => node({ kind: "delay", delayTime: param() }),
  } as unknown as BaseAudioContext;
  const chain = createZoneDspChain(context);
  chain.apply({
    enabled: true,
    preamp_db: -3,
    bass_db: 3,
    treble_db: -2,
    crossfeed: true,
  });
  const biquads = nodes.filter((item) => item.kind === "biquad") as {
    type: string;
    frequency: FakeParam;
    gain: FakeParam;
  }[];
  const lowshelf = biquads.find((item) => item.type === "lowshelf")!;
  const highshelf = biquads.find((item) => item.type === "highshelf")!;
  expect(lowshelf.frequency.value).toBe(100);
  expect(lowshelf.gain.target).toBe(3);
  expect(highshelf.frequency.value).toBe(10_000);
  expect(highshelf.gain.target).toBe(-2);
  expect(
    (chain.input as unknown as { gain: FakeParam }).gain.target,
  ).toBeCloseTo(10 ** (-3 / 20), 6);
  const feeds = nodes.filter(
    (item) =>
      item.kind === "gain" &&
      item !== (chain.input as unknown) &&
      item !== (chain.output as unknown),
  ) as { gain: FakeParam }[];
  expect(feeds).toHaveLength(2);
  for (const feed of feeds) expect(feed.gain.target).toBe(CROSSFEED_LEVEL);
  chain.apply(null);
  for (const feed of feeds) expect(feed.gain.target).toBe(0);
  expect(lowshelf.gain.target).toBe(0);
});
