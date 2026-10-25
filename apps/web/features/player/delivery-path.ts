import {
  deliveryQualityLabel,
  type DeliveryPlan,
  type SignalStep,
} from "@parson/music-sdk";

export type PathQuality = SignalStep["quality"];

export type PathRow = {
  key: string;
  stage: "source" | "server" | "player";
  title: string;
  detail: string;
  quality: PathQuality;
};

const STEP_TITLES: Record<string, string> = {
  source: "Source",
  passthrough: "Server",
  decode: "Decode",
  dsd_to_pcm: "DSD to PCM",
  dop: "DSD over PCM",
  resample: "Sample rate conversion",
  requantize: "Bit depth",
  downmix: "Downmix",
  effect: "Effect",
  encode: "Encode",
};

const QUALITY_ORDER: PathQuality[] = [
  "bit_perfect",
  "lossless",
  "high",
  "enhanced",
  "lossy",
];

/** Rows for the signal path: source, every server step, then this player. */
export function deliveryPathRows(
  plan: DeliveryPlan | null | undefined,
  player: { presetLabel?: string | null; casting?: boolean } = {},
): PathRow[] {
  if (!plan) return [];
  const rows: PathRow[] = plan.steps.map((step, index) => ({
    key: `${index}-${step.kind}`,
    stage: step.kind === "source" ? "source" : "server",
    title: STEP_TITLES[step.kind] ?? step.kind,
    detail: step.description,
    quality: step.quality,
  }));
  rows.push({
    key: "player-decode",
    stage: "player",
    title: player.casting ? "Cast receiver" : "This device",
    detail: `Decodes ${plan.output.name}${
      plan.mode === "lossy_transcode" ? "" : " without loss"
    }`,
    quality: plan.output.lossless === false ? "lossy" : "lossless",
  });
  if (player.presetLabel) {
    rows.push({
      key: "player-preset",
      stage: "player",
      title: "Sound preset",
      detail: player.presetLabel,
      quality: "enhanced",
    });
  }
  return rows;
}

/** The weakest link decides the light, as in a signal path. */
export function pathQuality(rows: PathRow[]): PathQuality {
  let worst: PathQuality = "bit_perfect";
  for (const row of rows) {
    if (QUALITY_ORDER.indexOf(row.quality) > QUALITY_ORDER.indexOf(worst)) {
      worst = row.quality;
    }
  }
  return worst;
}

export function pathQualityLabel(quality: PathQuality) {
  switch (quality) {
    case "bit_perfect":
      return deliveryQualityLabel("bit_perfect");
    case "lossless":
      return deliveryQualityLabel("lossless");
    case "high":
      return deliveryQualityLabel("high_quality");
    case "enhanced":
      return "Enhanced";
    default:
      return deliveryQualityLabel("lossy");
  }
}

export const PATH_QUALITY_DOT: Record<PathQuality, string> = {
  bit_perfect: "bg-violet-400 shadow-[0_0_10px_rgba(167,139,250,0.9)]",
  lossless: "bg-violet-400 shadow-[0_0_10px_rgba(167,139,250,0.9)]",
  high: "bg-sky-400 shadow-[0_0_10px_rgba(56,189,248,0.8)]",
  enhanced: "bg-emerald-400 shadow-[0_0_10px_rgba(52,211,153,0.8)]",
  lossy: "bg-amber-400 shadow-[0_0_10px_rgba(251,191,36,0.8)]",
};
