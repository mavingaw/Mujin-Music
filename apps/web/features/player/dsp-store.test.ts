import { describe, expect, test } from "bun:test";
import { defaultDspSettings } from "@parson/music-sdk";
import {
  audioOutputs,
  dspForOutput,
  outputDeviceLabel,
  outputKey,
  parseDspChains,
} from "./dsp-store";

describe("per-output DSP settings", () => {
  test("each output keeps its own chain and unknown ones start bypassed", () => {
    const usb = defaultDspSettings();
    usb.headroom = { enabled: true, gainDb: -4 };
    const chains = parseDspChains(JSON.stringify({ "usb-dac": usb }));
    expect(dspForOutput(chains, "usb-dac").headroom.gainDb).toBe(-4);
    expect(dspForOutput(chains, "")).toEqual(defaultDspSettings());
    expect(outputKey("default")).toBe("default");
    expect(outputKey("")).toBe("default");
  });

  test("broken storage is ignored", () => {
    expect(parseDspChains("{")).toEqual({});
    expect(parseDspChains("[1]")).toEqual({});
    expect(parseDspChains(null)).toEqual({});
  });
});

describe("output devices", () => {
  const devices = audioOutputs([
    {
      deviceId: "default",
      kind: "audiooutput",
      label: "Default - MacBook Pro Speakers",
    },
    {
      deviceId: "communications",
      kind: "audiooutput",
      label: "Communications - Headset",
    },
    { deviceId: "mic", kind: "audioinput", label: "Microphone" },
    { deviceId: "usb", kind: "audiooutput", label: "Schiit Modi+ (USB Audio)" },
    { deviceId: "usb", kind: "audiooutput", label: "Schiit Modi+ (USB Audio)" },
  ]);

  test("lists each output once, without input or communication devices", () => {
    expect(devices.map((device) => device.id)).toEqual(["default", "usb"]);
  });

  test("names the chosen device, or what the default points at", () => {
    expect(outputDeviceLabel(devices, "usb")).toBe("Schiit Modi+ (USB Audio)");
    expect(outputDeviceLabel(devices, "")).toBe("MacBook Pro Speakers");
    expect(outputDeviceLabel(devices, "gone")).toBe("MacBook Pro Speakers");
    expect(outputDeviceLabel([{ id: "default", label: "" }], "")).toBeNull();
  });
});
