import {
  dspEffectivePreamp,
  isDspActive,
  type ZoneDsp,
} from "@parson/music-sdk";

/** Shelf frequencies, matching the server's ffmpeg chain. */
export const DSP_BASS_HZ = 100;
export const DSP_TREBLE_HZ = 10_000;
/** Crossfeed: the opposite channel, low-passed, 300 µs late, -4.5 dB. */
export const CROSSFEED_CUTOFF_HZ = 700;
export const CROSSFEED_DELAY_S = 0.0003;
export const CROSSFEED_LEVEL = 10 ** (-4.5 / 20);

/** What the WebAudio DSP chain is set to for `dsp` (neutral when off). */
export function dspParameters(dsp: ZoneDsp | null | undefined): {
  active: boolean;
  preampGain: number;
  bassDb: number;
  trebleDb: number;
  crossfeed: boolean;
} {
  if (!dsp || !isDspActive(dsp)) {
    return {
      active: false,
      preampGain: 1,
      bassDb: 0,
      trebleDb: 0,
      crossfeed: false,
    };
  }
  return {
    active: true,
    preampGain: 10 ** (dspEffectivePreamp(dsp) / 20),
    bassDb: dsp.bass_db,
    trebleDb: dsp.treble_db,
    crossfeed: dsp.crossfeed,
  };
}

/** Per-zone DSP in the browser: preamp, bass/treble shelves, crossfeed. */
export type DspChain = {
  input: GainNode;
  output: GainNode;
  apply: (dsp: ZoneDsp | null | undefined) => void;
};

export function createDspChain(context: BaseAudioContext): DspChain {
  const input = context.createGain();
  const bass = context.createBiquadFilter();
  bass.type = "lowshelf";
  bass.frequency.value = DSP_BASS_HZ;
  const treble = context.createBiquadFilter();
  treble.type = "highshelf";
  treble.frequency.value = DSP_TREBLE_HZ;
  const output = context.createGain();
  input.connect(bass);
  bass.connect(treble);
  treble.connect(output);

  // Crossfeed: each ear also hears a filtered, delayed copy of the other.
  const splitter = context.createChannelSplitter(2);
  const merger = context.createChannelMerger(2);
  const feeds = [0, 1].map(() => {
    const filter = context.createBiquadFilter();
    filter.type = "lowpass";
    filter.frequency.value = CROSSFEED_CUTOFF_HZ;
    const delay = context.createDelay(0.01);
    delay.delayTime.value = CROSSFEED_DELAY_S;
    const level = context.createGain();
    level.gain.value = 0;
    filter.connect(delay);
    delay.connect(level);
    return { filter, level };
  });
  treble.connect(splitter);
  splitter.connect(feeds[0]!.filter, 0);
  splitter.connect(feeds[1]!.filter, 1);
  // Left feeds the right ear, right feeds the left ear.
  feeds[0]!.level.connect(merger, 0, 1);
  feeds[1]!.level.connect(merger, 0, 0);
  merger.connect(output);

  return {
    input,
    output,
    apply(dsp) {
      const parameters = dspParameters(dsp);
      const now = context.currentTime;
      input.gain.setTargetAtTime(parameters.preampGain, now, 0.02);
      bass.gain.setTargetAtTime(parameters.bassDb, now, 0.02);
      treble.gain.setTargetAtTime(parameters.trebleDb, now, 0.02);
      for (const feed of feeds) {
        feed.level.gain.setTargetAtTime(
          parameters.crossfeed ? CROSSFEED_LEVEL : 0,
          now,
          0.02,
        );
      }
    },
  };
}
