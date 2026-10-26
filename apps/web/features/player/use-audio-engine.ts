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
  activeConvolutionFilter,
  createDspChain,
  defaultDspSettings,
  dspNeedsGraph,
  getConvolutionFilterFile,
  requestedContextSampleRate,
  type DspAudioContext,
  type DspSettings,
  type NativeOutputLeveling,
  type NativeOutputSession,
} from "@parson/music-sdk";
import { NativeOutputElement } from "./native-output-element";
import { crossfadeGains, leveledVolume } from "./crossfade";
import { createZoneDspChain, dspParameters } from "./zone-dsp";
import type { ZoneDsp } from "@parson/music-sdk";

const CROSSFADE_TICK_MS = 50;

type Crossfade = {
  outgoing: HTMLAudioElement;
  /** The outgoing element's audio graph, closed when the fade ends. */
  outgoingGraph: AudioGraph | null;
  startVolume: number;
  startedAt: number;
  seconds: number;
  timer: ReturnType<typeof setInterval>;
};

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
import { startedStream } from "@/lib/api/stream-url";

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
  // Linear volume-leveling gain from the track's ReplayGain tags, folded
  // into the element volume.
  const levelingGainRef = useRef(1);
  /**
   * Leveling factor from the zone's measured loudness (zone setting). When
   * the zone levels, it replaces the tag-based gain; `null` when it does not.
   */
  const zoneLevelFactor = useRef<number | null>(null);
  /** Zone crossfade time in seconds (0: off). */
  const crossfadeSeconds = useRef(0);
  /** Zone DSP for this device (preamp, shelves, crossfeed). */
  const zoneDspRef = useRef<ZoneDsp | null>(null);
  /** The next source change should crossfade instead of cutting. */
  const crossfadeArmed = useRef(false);
  const crossfade = useRef<Crossfade | null>(null);
  const mutedRef = useRef(false);
  const recovered = useRef(false);
  const presetRef = useRef<AudioPreset>(audioPresets[0]!);
  const [audioVersion, setAudioVersion] = useState(0);
  // The stream now playing and how many times its first frame has been
  // decoded, so the signal path re-reads what this stream delivered.
  const [stream, setStream] = useState({ url: "", frames: 0 });
  const [audioPreset, setAudioPresetState] =
    useState<AudioPresetId>("original");
  const [isPlaying, setIsPlaying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);
  const [graphSampleRate, setGraphSampleRate] = useState<number | null>(null);
  // Convolution filters decoded for the graph, keyed `id@rate` (a
  // ConvolverNode needs a buffer at its context's rate).
  const impulses = useRef(new Map<string, AudioBuffer>());
  const impulseLoading = useRef(new Set<string>());
  const [loadedImpulse, setLoadedImpulse] = useState<string | null>(null);
  const dspRef = useRef<DspSettings>(defaultDspSettings());
  const sourceRateRef = useRef<number | null>(null);
  const sinkIdRef = useRef("");
  const [activeSinkId, setActiveSinkId] = useState("");
  const [deviceSampleRate, setDeviceSampleRate] = useState<number | null>(null);
  // Set while a server output plays: `audio.current` is then a
  // NativeOutputElement and nothing is decoded in the browser.
  const nativeRef = useRef<NativeOutputElement | null>(null);
  const nativeLevelingRef = useRef<NativeOutputLeveling | null>(null);
  const [nativeSession, setNativeSession] =
    useState<NativeOutputSession | null>(null);
  const needsGraph = useCallback(
    () =>
      presetRef.current.wet > 0 ||
      dspNeedsGraph(dspRef.current, sourceRateRef.current) ||
      dspParameters(zoneDspRef.current).active,
    [],
  );
  const wantedRate = useCallback(
    () => requestedContextSampleRate(dspRef.current, sourceRateRef.current),
    [],
  );
  const elementVolume = useCallback(
    () =>
      leveledVolume(
        volumeRef.current,
        zoneLevelFactor.current ?? levelingGainRef.current,
      ),
    [],
  );

  /** Ends a running crossfade now: the outgoing track stops. */
  const finishCrossfade = useCallback(() => {
    const fade = crossfade.current;
    if (!fade) return;
    crossfade.current = null;
    clearInterval(fade.timer);
    fade.outgoing.pause();
    fade.outgoing.removeAttribute("src");
    fade.outgoing.load();
    if (fade.outgoingGraph) {
      fade.outgoingGraph.chain.dispose();
      void fade.outgoingGraph.context.close().catch(() => {});
    }
    if (audio.current) audio.current.volume = elementVolume();
  }, [elementVolume]);

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
      if (!next || typeof window === "undefined" || nativeRef.current) {
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

  /**
   * Fetches and decodes the chosen convolution filter for the graph's
   * rate, then rewires the chain with it.
   */
  const loadImpulse = useCallback((settings: DspSettings) => {
    const id = activeConvolutionFilter(settings);
    const current = graph.current;
    if (!id || !current) return;
    const key = `${id}@${current.context.sampleRate}`;
    if (impulses.current.has(key)) {
      setLoadedImpulse(id);
      return;
    }
    if (impulseLoading.current.has(key)) return;
    impulseLoading.current.add(key);
    void getConvolutionFilterFile(id)
      .then((data) => current.context.decodeAudioData(data))
      .then((buffer) => {
        impulses.current.set(key, buffer);
        if (graph.current === current) {
          current.chain.update(dspRef.current);
          setLoadedImpulse(id);
        }
      })
      .catch((cause) => {
        console.error("Unable to load the convolution filter", cause);
      })
      .finally(() => impulseLoading.current.delete(key));
  }, []);

  const ensureAudioGraph = useCallback(() => {
    if (nativeRef.current) return null;
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
        {
          impulse: (id) =>
            impulses.current.get(`${id}@${context.sampleRate}`) ?? null,
        },
      );
      const chainOutput = chain.output as unknown as AudioNode;
      const dryGain = context.createGain();
      const wetGain = context.createGain();
      const convolver = context.createConvolver();
      convolver.buffer = buildReverbImpulse(context);
      // Per-zone DSP runs after the preset mix, just before the speakers.
      const zoneDsp = createZoneDspChain(context);
      mediaSource.connect(chain.input as unknown as AudioNode);
      chainOutput.connect(dryGain);
      convolver.connect(wetGain);
      dryGain.connect(zoneDsp.input);
      wetGain.connect(zoneDsp.input);
      zoneDsp.output.connect(context.destination);
      dryGain.gain.value = 1;
      wetGain.gain.value = 0;
      zoneDsp.apply(zoneDspRef.current);
      graph.current = {
        chain,
        convolver,
        context,
        dryGain,
        requestedRate: rate,
        source: mediaSource,
        wetConnected: false,
        wetGain,
        zoneDsp,
      };
      setGraphSampleRate(context.sampleRate);
      loadImpulse(dspRef.current);
    } catch (cause) {
      console.error("Unable to initialize the audio processing graph", cause);
      return null;
    }
    return graph.current;
  }, [loadImpulse, recoverAudioElement, wantedRate]);

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
      audioGraph.zoneDsp?.apply(zoneDspRef.current);
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
      const armed = crossfadeArmed.current;
      crossfadeArmed.current = false;
      // Any running fade ends before the source changes again.
      finishCrossfade();
      startedStream(next);
      setStream({ url: next, frames: 0 });
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
        const previousGraph = graph.current;
        graph.current = null;
        setGraphSampleRate(null);
        const remaining =
          Number.isFinite(previous.duration) && previous.duration > 0
            ? previous.duration - previous.currentTime
            : 0;
        const seconds = Math.min(crossfadeSeconds.current, remaining);
        if (armed && !previous.paused && seconds > 0.2) {
          // Crossfade: the outgoing track keeps playing and fades out
          // while the new one fades in (equal power).
          prepared.volume = 0;
          crossfade.current = {
            outgoing: previous,
            outgoingGraph: previousGraph,
            startVolume: previous.volume,
            startedAt: performance.now(),
            seconds,
            timer: setInterval(() => {
              const current = crossfade.current;
              if (!current) return;
              const progress =
                (performance.now() - current.startedAt) /
                (current.seconds * 1000);
              const gains = crossfadeGains(progress);
              current.outgoing.volume = current.startVolume * gains.outgoing;
              if (audio.current) {
                audio.current.volume = elementVolume() * gains.incoming;
              }
              if (progress >= 1) finishCrossfade();
            }, CROSSFADE_TICK_MS),
          };
        } else {
          previous.pause();
          previous.removeAttribute("src");
          previous.load();
          if (previousGraph) {
            previousGraph.chain.dispose();
            void previousGraph.context.close().catch(() => {});
          }
        }
        setAudioVersion((version) => version + 1);
        return;
      }

      discardPreloadedAudio();
      configureAudioElement(audio.current);
      audio.current.src = next;
    },
    [
      configureAudioElement,
      discardPreloadedAudio,
      dropAudioGraph,
      elementVolume,
      finishCrossfade,
      wantedRate,
    ],
  );

  /**
   * Asks the next source change to crossfade. Only when a crossfade time is
   * set and the next track is already buffered (else it would stall).
   */
  const armCrossfade = useCallback(() => {
    const prepared = preloadedAudio.current;
    if (
      crossfadeSeconds.current <= 0 ||
      crossfade.current ||
      !prepared ||
      !canPromotePreloadedMedia(
        prepared,
        preloadedSource.current,
        preloadedSource.current,
      )
    ) {
      return false;
    }
    crossfadeArmed.current = true;
    return true;
  }, []);

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

  /**
   * Plays through a server output (`deviceId`) or back through this
   * browser (`null`), keeping the track, position and play state.
   */
  const setNativeOutput = useCallback(
    (deviceId: string | null) => {
      const current = nativeRef.current;
      if ((current?.deviceId ?? null) === deviceId) return;
      if (typeof window === "undefined") return;
      finishCrossfade();
      const element = audio.current;
      const wasPlaying = Boolean(element && !element.paused);
      const position = element?.currentTime ?? 0;
      discardPreloadedAudio();
      if (graph.current) {
        graph.current.chain.dispose();
        void graph.current.context.close().catch(() => {});
        graph.current = null;
        setGraphSampleRate(null);
      }
      if (current) {
        current.dispose();
      } else if (element) {
        element.pause();
        element.removeAttribute("src");
        element.load();
      }
      playbackAttempt.current += 1;
      let next: HTMLAudioElement;
      if (deviceId) {
        const native = new NativeOutputElement(deviceId, {
          onSession: setNativeSession,
        });
        native.leveling = nativeLevelingRef.current;
        nativeRef.current = native;
        if (source.current) native.src = source.current;
        native.currentTime = position;
        next = native as unknown as HTMLAudioElement;
      } else {
        nativeRef.current = null;
        setNativeSession(null);
        next = createAudioElement();
        configureAudioElement(next);
        const sinkTarget = next as HTMLAudioElement & SinkTarget;
        if (sinkIdRef.current && sinkTarget.setSinkId)
          void sinkTarget.setSinkId(sinkIdRef.current).catch(() => {});
        if (source.current) {
          next.addEventListener(
            "loadedmetadata",
            () => {
              try {
                if (position > 0) next.currentTime = position;
              } catch {}
            },
            { once: true },
          );
          next.src = source.current;
        }
      }
      audio.current = next;
      recovered.current = true;
      setIsPlaying(false);
      setAudioVersion((version) => version + 1);
      if (wasPlaying) window.setTimeout(() => playAudioSource(), 0);
    },
    [
      configureAudioElement,
      discardPreloadedAudio,
      finishCrossfade,
      playAudioSource,
    ],
  );

  useEffect(() => () => nativeRef.current?.dispose(), []);

  /** Leveling the server applies while a server output plays. */
  const setNativeLeveling = useCallback(
    (leveling: NativeOutputLeveling | null) => {
      nativeLevelingRef.current = leveling;
      if (nativeRef.current) nativeRef.current.leveling = leveling;
    },
    [],
  );

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
        loadImpulse(settings);
        return;
      }
      if (needsGraph() && audio.current && !audio.current.paused) {
        const created = ensureAudioGraph();
        if (created?.context.state === "suspended")
          void created.context.resume().catch(() => {});
        applyPreset(presetRef.current);
      }
    },
    [
      applyPreset,
      ensureAudioGraph,
      loadImpulse,
      needsGraph,
      resetAudioGraph,
      wantedRate,
    ],
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

  /**
   * Zone settings for this device: crossfade time, the zone's leveling
   * factor (`null` when the zone does not level) and its DSP.
   */
  const setOutputSettings = useCallback(
    ({
      crossfadeSeconds: seconds,
      levelingFactor: factor,
      dsp,
    }: {
      crossfadeSeconds: number;
      levelingFactor: number | null;
      dsp?: ZoneDsp | null;
    }) => {
      crossfadeSeconds.current = Math.max(0, seconds);
      zoneLevelFactor.current =
        factor !== null && Number.isFinite(factor) && factor > 0
          ? factor
          : null;
      if (audio.current && !crossfade.current) {
        audio.current.volume = elementVolume();
      }
      zoneDspRef.current = dsp ?? null;
      if (nativeRef.current) return;
      const audioGraph =
        graph.current ??
        (dspParameters(dsp).active ? ensureAudioGraph() : null);
      audioGraph?.zoneDsp?.apply(zoneDspRef.current);
    },
    [elementVolume, ensureAudioGraph],
  );

  const setAudioVolume = useCallback(
    (value: number | string) => {
      const next = boundedVolume(value);
      if (next === null) return;
      volumeRef.current = next;
      if (audio.current && !crossfade.current)
        audio.current.volume = elementVolume();
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
      if (audio.current && !crossfade.current)
        audio.current.volume = elementVolume();
    },
    [elementVolume],
  );
  const toggleMute = useCallback(() => {
    if (!audio.current) return;
    audio.current.muted = !audio.current.muted;
    mutedRef.current = audio.current.muted;
    if (preloadedAudio.current) preloadedAudio.current.muted = mutedRef.current;
    if (crossfade.current) crossfade.current.outgoing.muted = mutedRef.current;
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
  useEffect(() => finishCrossfade, [finishCrossfade]);

  useEffect(() => {
    const element = audio.current;
    if (!element) return;
    const firstFrame = () =>
      setStream((current) =>
        current.url === source.current
          ? { ...current, frames: current.frames + 1 }
          : { url: source.current, frames: 1 },
      );
    element.addEventListener("loadeddata", firstFrame);
    return () => element.removeEventListener("loadeddata", firstFrame);
  }, [audioVersion]);

  return {
    activeSinkId,
    armCrossfade,
    audio,
    audioPreset,
    audioVersion,
    backgroundWasPlaying,
    error,
    finishCrossfade,
    graph,
    deviceSampleRate,
    graphSampleRate,
    isPlaying,
    loadedImpulse,
    muted,
    nativeSession,
    playAudioSource,
    preloadAudioSource,
    reapplyOutputDevice,
    resumeOnReconnect,
    setAudioPreset,
    setDspSettings,
    setNativeLeveling,
    setNativeOutput,
    setOutputDevice,
    setOutputSettings,
    setAudioSource,
    setError,
    setIsPlaying,
    setAudioVolume,
    setLevelingGain,
    slowedReverb: audioPreset !== "original",
    source,
    stream,
    toggleMute,
    toggleSlowedReverb,
    volume,
  };
}
