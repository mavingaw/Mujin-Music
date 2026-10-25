import type { Zone, ZoneStreamInfo } from "@parson/music-sdk";

export type SignalStageIcon =
  "source" | "rate" | "depth" | "gain" | "dsp" | "decode" | "encode" | "output";

export interface SignalStage {
  /** `Source` and `Output` stages carry a kicker; steps do not. */
  kicker: "Source" | "Output" | null;
  label: string;
  icon: SignalStageIcon;
}

/** The icon for one processing step, from the server's wording. */
export function stepIcon(step: string): SignalStageIcon {
  if (step.startsWith("Sample rate") || step.startsWith("Resampled"))
    return "rate";
  if (
    step.startsWith("Bit depth") ||
    step.startsWith("Carried at") ||
    step.startsWith("Reduced to")
  )
    return "depth";
  if (step.startsWith("Volume leveling") || step.startsWith("Software volume"))
    return "gain";
  if (step.startsWith("DSP")) return "dsp";
  if (step.startsWith("Decoded")) return "decode";
  return "encode";
}

/** Source, each processing step in the order it runs, then the output. */
export function signalPathStages(stream: ZoneStreamInfo): SignalStage[] {
  return [
    { kicker: "Source", label: stream.source, icon: "source" },
    ...stream.steps.map((step) => ({
      kicker: null,
      label: step,
      icon: stepIcon(step),
    })),
    { kicker: "Output", label: stream.output, icon: "output" },
  ];
}

/**
 * What Parson cannot see past the end of the path, and what each output
 * type has (and has not) been tested against.
 */
export function signalPathCaveat(zone: Pick<Zone, "kind">): string | null {
  switch (zone.kind) {
    case "sonos":
      return "Anything the speaker does after it receives the stream (Trueplay, EQ, its own volume) happens inside the Sonos and is not shown. This path comes from the exact conversion plan behind the stream URL Sonos is given. Parson's Sonos support is tested against a protocol-level Sonos simulator, not yet on Sonos hardware.";
    case "upnp":
      return "What the renderer does after it receives the stream is not shown. UPnP support is tested against a protocol-level renderer simulator, not yet on renderer hardware.";
    case "chromecast":
      return "What the Cast device does after it receives the stream is not shown. Cast support is tested against a simulated receiver, not yet on Cast hardware.";
    case "device":
      return "The DAC's own filters and analog stage are not shown. Device output is tested against simulated DACs and ALSA's null device, not yet on a USB DAC.";
    default:
      return null;
  }
}
