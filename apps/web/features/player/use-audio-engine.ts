"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { boundedVolume } from "./player-state";
import {
  audioPresets,
  buildReverbImpulse,
  createAudioElement,
  defaultAudioPreset,
  getAudioPreset,
  setPitchPreservation,
  type AudioGraph,
  type AudioPreset,
  type AudioPresetId,
} from "./audio-presets";
import { canPromotePreloadedMedia } from "./gapless-playback";
import {
  createDspChain,
  defaultDspSettings,
  dspNeedsGraph,
  requestedContextSampleRate,
  type DspAudioContext,
  type DspSettings,
} from "@parson/music-sdk";

type SinkTarget = { setSinkId?: (sinkId: string) => Promise<void> };

function audioContextConstructor() {
  if (typeof window === "undefined") return null;
  return (
    window.AudioContext ||
    (window as typeof window & { webkitAudioContext?: typeof AudioContext })
      .webkitAudioContext ||
    null
  );
}

/**
 * The output device's own rate: a context created without a rate runs at
 * it. `sinkId` targets a chosen device where the browser supports it.
 */
export async function probeDeviceSampleRate(
  sinkId: string,
): Promise<number | null> {
  const Constructor = audioContextConstructor();
  if (!Constructor) return null;
  try {
    const context = new Constructor(
      sinkId ? ({ sinkId } as AudioContextOptions) : undefined,
    );
    const rate = context.sampleRate;
    await context.close().catch(() => {});
    return Number.isFinite(rate) && rate > 0 ? rate : null;
  } catch {
    return null;
  }
}

const PLAYBACK_START_TIMEOUT_MS = 12_000;

export function useAudioEngine() {
  const audio = useRef<HTMLAudioElement | null>(
    typeof window === "undefined" ? null : createAudioElement(),
  );
  const preloadedAudio = useRef<HTMLAudioElement | null>(null);
  const preloadedSource = useRef("");
  const graph = useRef<AudioGraph | null>(null);
  const source = useRef("");
  const playbackAttempt = useRef(0);
  const resumeOnReconnect = useRef(false);
  const backgroundWasPlaying = useRef(false);
  const volumeRef = useRef(1);
  // Linear volume-leveling gain, folded into the element volume.
  const levelingGainRef = useRef(1);
  const mutedRef = useRef(false);
  const recovered = useRef(false);
  const presetRef = useRef<AudioPreset>(audioPresets[0]!);
  const [audioVersion, setAudioVersion] = useState(0);
  const [audioPreset, setAudioPresetState] =
    useState<AudioPresetId>("original");
  const [isPlaying, setIsPlaying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);
  const [graphSampleRate, setGraphSampleRate] = useState<number | null>(null);
  const dspRef = useRef<DspSettings>(defaultDspSettings());
  const sourceRateRef = useRef<number | null>(null);
  const sinkIdRef = useRef("");
  const [activeSinkId, setActiveSinkId] = useState("");
  const [deviceSampleRate, setDeviceSampleRate] = useState<number | null>(null);
  const needsGraph = useCallback(
    () =>
      presetRef.current.wet > 0 ||
      dspNeedsGraph(dspRef.current, sourceRateRef.current),
    [],
  );
  const wantedRate = useCallback(
    () => requestedContextSampleRate(dspRef.current, sourceRateRef.current),
    [],
  );
  const elementVolume = useCallback(
    () => Math.min(1, Math.max(0, volumeRef.current * levelingGainRef.current)),
    [],
  );

  const discardPreloadedAudio = useCallback(() => {
    const element = preloadedAudio.current;
    element?.pause();
    element?.removeAttribute("src");
    element?.load();
    preloadedAudio.current = null;
    preloadedSource.current = "";
  }, []);

  const configureAudioElement = useCallback(
    (element: HTMLAudioElement) => {
      element.crossOrigin = "anonymous";
      element.volume = elementVolume();
      element.muted = mutedRef.current;
      setPitchPreservation(element, presetRef.current.preservePitch);
      element.defaultPlaybackRate = presetRef.current.rate;
      element.playbackRate = presetRef.current.rate;
    },
    [elementVolume],
  );

  const preloadAudioSource = useCallback(
    (next: string | null) => {
      if (!next || typeof window === "undefined") {
        discardPreloadedAudio();
        return;
      }
      if (preloadedAudio.current && preloadedSource.current === next) return;
      discardPreloadedAudio();
      const element = createAudioElement();
      configureAudioElement(element);
      const sinkTarget = element as HTMLAudioElement & SinkTarget;
      if (sinkIdRef.current && sinkTarget.setSinkId)
        void sinkTarget.setSinkId(sinkIdRef.current).catch(() => {});
      element.preload = "auto";
      element.src = next;
      element.load();
      preloadedAudio.current = element;
      preloadedSource.current = next;
    },
    [configureAudioElement, discardPreloadedAudio],
  );

  const recoverAudioElement = useCallback(() => {
    if (recovered.current || typeof window === "undefined") return;
    const current = audio.current;
    const resumeAt = current?.currentTime || 0;
    current?.pause();
    const replacement = createAudioElement();
    configureAudioElement(replacement);
    if (source.current) {
      replacement.src = source.current;
      try {
        replacement.currentTime = resumeAt;
      } catch {}
      replacement.load();
    }
    audio.current = replacement;
    recovered.current = true;
    setAudioVersion((version) => version + 1);
  }, [configureAudioElement]);

  const ensureAudioGraph = useCallback(() => {
    recoverAudioElement();
    const element = audio.current;
    if (!element || graph.current || typeof window === "undefined")
      return graph.current;
    const Constructor = audioContextConstructor();
    if (!Constructor) return null;
    try {
      const rate = wantedRate();
      let context: AudioContext;
      try {
        context = rate
          ? new Constructor({ sampleRate: rate })
          : new Constructor();
      } catch {
        // Rates outside the browser's range fall back to the device rate.
        context = new Constructor();
      }
      const sinkId = sinkIdRef.current;
      const sinkContext = context as AudioContext & SinkTarget;
      if (sinkId && sinkContext.setSinkId)
        void sinkContext.setSinkId(sinkId).catch(() => {});
      const mediaSource = context.createMediaElementSource(element);
      const chain = createDspChain(
        context as unknown as DspAudioContext,
        dspRef.current,
      );
      const chainOutput = chain.output as unknown as AudioNode;
      const dryGain = context.createGain();
      const wetGain = context.createGain();
      const convolver = context.createConvolver();
      convolver.buffer = buildReverbImpulse(context);
      mediaSource.connect(chain.input as unknown as AudioNode);
      chainOutput.connect(dryGain);
      convolver.connect(wetGain);
      dryGain.connect(context.destination);
      wetGain.connect(context.destination);
      dryGain.gain.value = 1;
      wetGain.gain.value = 0;
      graph.current = {
        chain,
        convolver,
        context,
        dryGain,
        requestedRate: rate,
        source: mediaSource,
        wetConnected: false,
        wetGain,
      };
      setGraphSampleRate(context.sampleRate);
    } catch (cause) {
      console.error("Unable to initialize the audio processing graph", cause);
      return null;
    }
    return graph.current;
  }, [recoverAudioElement, wantedRate]);

  /** Closes the graph; the element cannot be reused, so it is replaced. */
  const dropAudioGraph = useCallback(
    (keepPosition = true) => {
      const current = graph.current;
      if (!current) return false;
      current.chain.dispose();
      void current.context.close().catch(() => {});
      graph.current = null;
      setGraphSampleRate(null);
      const element = audio.current;
      const replacement = createAudioElement();
      configureAudioElement(replacement);
      const sinkTarget = replacement as HTMLAudioElement & SinkTarget;
      if (sinkIdRef.current && sinkTarget.setSinkId)
        void sinkTarget.setSinkId(sinkIdRef.current).catch(() => {});
      const resumeAt = keepPosition ? (element?.currentTime ?? 0) : 0;
      if (element) {
        element.pause();
        element.removeAttribute("src");
        element.load();
      }
      if (source.current) {
        replacement.addEventListener(
          "loadedmetadata",
          () => {
            try {
              if (resumeAt > 0) replacement.currentTime = resumeAt;
            } catch {}
          },
          { once: true },
        );
        replacement.src = source.current;
      }
      audio.current = replacement;
      recovered.current = true;
      setAudioVersion((version) => version + 1);
      return true;
    },
    [configureAudioElement],
  );

  const applyPreset = useCallback(
    (preset: AudioPreset) => {
      const element = audio.current;
      if (!element) return;
      setPitchPreservation(element, preset.preservePitch);
      element.defaultPlaybackRate = preset.rate;
      element.playbackRate = preset.rate;
      const audioGraph =
        graph.current ?? (needsGraph() ? ensureAudioGraph() : null);
      if (!audioGraph) return;
      const chainOutput = audioGraph.chain.output as unknown as AudioNode;
      if (preset.wet > 0 && !audioGraph.wetConnected) {
        chainOutput.connect(audioGraph.convolver);
        audioGraph.wetConnected = true;
      } else if (preset.wet <= 0 && audioGraph.wetConnected) {
        try {
          chainOutput.disconnect(audioGraph.convolver);
        } catch {}
        audioGraph.wetConnected = false;
      }
      const now = audioGraph.context.currentTime;
      audioGraph.wetGain.gain.setTargetAtTime(preset.wet, now, 0.025);
      audioGraph.dryGain.gain.setTargetAtTime(preset.dry, now, 0.025);
    },
    [ensureAudioGraph, needsGraph],
  );

  const setAudioSource = useCallback(
    (next: string, sourceSampleRate?: number | null) => {
      playbackAttempt.current += 1;
      setIsPlaying(false);
      setError(null);
      source.current = next;
      sourceRateRef.current = sourceSampleRate ?? null;
      if (!audio.current) return;
      // "Match source" needs a context at this track's rate.
      if (graph.current && graph.current.requestedRate !== wantedRate())
        dropAudioGraph(false);

      const prepared = preloadedAudio.current;
      if (
        prepared &&
        canPromotePreloadedMedia(prepared, preloadedSource.current, next)
      ) {
        const previous = audio.current;
        audio.current = prepared;
        preloadedAudio.current = null;
        preloadedSource.current = "";
        configureAudioElement(prepared);
        previous.pause();
        previous.removeAttribute("src");
        previous.load();
        if (graph.current) {
          void graph.current.context.close().catch(() => {});
          graph.current = null;
          setGraphSampleRate(null);
        }
        setAudioVersion((version) => version + 1);
        return;
      }

      discardPreloadedAudio();
      configureAudioElement(audio.current);
      audio.current.src = next;
    },
    [configureAudioElement, discardPreloadedAudio, dropAudioGraph, wantedRate],
  );

  const playAudioSource = useCallback(() => {
    recoverAudioElement();
    const element = audio.current;
    if (!element) return;
    applyPreset(presetRef.current);
    if (graph.current?.context.state === "suspended")
      void graph.current.context.resume().catch(() => {});
    if (source.current && element.src !== source.current) {
      element.crossOrigin = "anonymous";
      element.src = source.current;
      element.load();
    }
    const attempt = ++playbackAttempt.current;
    const attemptedSource = source.current;
    const startTimeout = window.setTimeout(() => {
      if (
        attempt !== playbackAttempt.current ||
        attemptedSource !== source.current ||
        !element.paused
      )
        return;
      playbackAttempt.current += 1;
      element.pause();
      setIsPlaying(false);
      setError(
        "Playback did not start. Check that Linux media codecs are installed, then try again.",
      );
    }, PLAYBACK_START_TIMEOUT_MS);
    void element
      .play()
      .then(
        () => {
          if (
            attempt === playbackAttempt.current &&
            attemptedSource === source.current
          ) {
            setIsPlaying(true);
            setError(null);
            resumeOnReconnect.current = false;
          }
        },
        (cause: unknown) => {
          if (
            attempt !== playbackAttempt.current ||
            attemptedSource !== source.current ||
            (cause instanceof DOMException && cause.name === "AbortError")
          )
            return;
          setIsPlaying(false);
          if (typeof navigator !== "undefined" && !navigator.onLine)
            resumeOnReconnect.current = true;
          setError(
            cause instanceof DOMException && cause.name === "NotAllowedError"
              ? "Your browser blocked audio. Allow sound for this site, then press play."
              : cause instanceof DOMException &&
                  cause.name === "NotSupportedError"
                ? "This audio format isn’t supported on this device. Try another track or convert the file."
                : "This track could not be played.",
          );
        },
      )
      .finally(() => window.clearTimeout(startTimeout));
  }, [applyPreset, recoverAudioElement]);

  /** Rebuilds the playback path after a graph change, keeping position. */
  const resetAudioGraph = useCallback(() => {
    const wasPlaying = Boolean(audio.current && !audio.current.paused);
    if (!dropAudioGraph()) return;
    if (wasPlaying) playAudioSource();
    else if (needsGraph()) applyPreset(presetRef.current);
  }, [applyPreset, dropAudioGraph, needsGraph, playAudioSource]);

  /** Applies DSP settings: rewires the chain or rebuilds the graph. */
  const setDspSettings = useCallback(
    (settings: DspSettings) => {
      dspRef.current = settings;
      const current = graph.current;
      if (current) {
        if (!needsGraph() || current.requestedRate !== wantedRate()) {
          resetAudioGraph();
          return;
        }
        current.chain.update(settings);
        return;
      }
      if (needsGraph() && audio.current && !audio.current.paused) {
        const created = ensureAudioGraph();
        if (created?.context.state === "suspended")
          void created.context.resume().catch(() => {});
        applyPreset(presetRef.current);
      }
    },
    [applyPreset, ensureAudioGraph, needsGraph, resetAudioGraph, wantedRate],
  );

  /** Routes playback to an output device; `""` is the system default. */
  const setOutputDevice = useCallback(async (sinkId: string) => {
    sinkIdRef.current = sinkId;
    const targets: SinkTarget[] = [
      audio.current,
      preloadedAudio.current,
      graph.current?.context,
    ].filter(Boolean) as SinkTarget[];
    let applied = sinkId;
    for (const target of targets) {
      if (!target.setSinkId) continue;
      try {
        await target.setSinkId(sinkId);
      } catch {
        // The device went away or is not allowed: use the default.
        applied = "";
        await target.setSinkId("").catch(() => {});
      }
    }
    if (!targets.some((target) => target.setSinkId)) applied = "";
    setActiveSinkId(applied);
    setDeviceSampleRate(await probeDeviceSampleRate(applied));
  }, []);

  const reapplyOutputDevice = useCallback(
    () => setOutputDevice(sinkIdRef.current),
    [setOutputDevice],
  );

  const setAudioVolume = useCallback(
    (value: number | string) => {
      const next = boundedVolume(value);
      if (next === null) return;
      volumeRef.current = next;
      if (audio.current) audio.current.volume = elementVolume();
      if (preloadedAudio.current)
        preloadedAudio.current.volume = elementVolume();
      setVolume(next);
      if (next > 0) {
        mutedRef.current = false;
        if (audio.current) audio.current.muted = false;
        if (preloadedAudio.current) preloadedAudio.current.muted = false;
        setMuted(false);
      }
    },
    [elementVolume],
  );
  /** Applies a linear volume-leveling gain to the playing element. */
  const setLevelingGain = useCallback(
    (gain: number) => {
      levelingGainRef.current = Number.isFinite(gain) && gain > 0 ? gain : 1;
      if (audio.current) audio.current.volume = elementVolume();
    },
    [elementVolume],
  );
  const toggleMute = useCallback(() => {
    if (!audio.current) return;
    audio.current.muted = !audio.current.muted;
    mutedRef.current = audio.current.muted;
    if (preloadedAudio.current) preloadedAudio.current.muted = mutedRef.current;
    setMuted(audio.current.muted);
  }, []);
  const setAudioPreset = useCallback(
    (id: AudioPresetId) => {
      const preset = getAudioPreset(id);
      presetRef.current = preset;
      setAudioPresetState(preset.id);
      if (graph.current && !needsGraph()) {
        // Nothing needs the graph: go back to the element's direct path.
        resetAudioGraph();
        return;
      }
      applyPreset(preset);
      if (preloadedAudio.current) configureAudioElement(preloadedAudio.current);
      if (graph.current?.context.state === "suspended")
        void graph.current.context.resume().catch(() => {});
    },
    [applyPreset, configureAudioElement, needsGraph, resetAudioGraph],
  );
  const toggleSlowedReverb = useCallback(
    () =>
      setAudioPreset(
        presetRef.current.id === "original"
          ? defaultAudioPreset.id
          : "original",
      ),
    [setAudioPreset],
  );

  useEffect(() => {
    if (graph.current?.context.state === "suspended")
      void graph.current.context.resume().catch(() => {});
  }, [audioPreset]);

  useEffect(() => discardPreloadedAudio, [discardPreloadedAudio]);

  return {
    activeSinkId,
    audio,
    audioPreset,
    audioVersion,
    backgroundWasPlaying,
    error,
    graph,
    deviceSampleRate,
    graphSampleRate,
    isPlaying,
    muted,
    playAudioSource,
    preloadAudioSource,
    reapplyOutputDevice,
    resumeOnReconnect,
    setAudioPreset,
    setDspSettings,
    setOutputDevice,
    setAudioSource,
    setError,
    setIsPlaying,
    setAudioVolume,
    setLevelingGain,
    slowedReverb: audioPreset !== "original",
    source,
    toggleMute,
    toggleSlowedReverb,
    volume,
  };
}
