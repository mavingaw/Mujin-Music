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
import { crossfadeGains, leveledVolume } from "./crossfade";
import { createDspChain, dspParameters } from "./zone-dsp";
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
  const mutedRef = useRef(false);
  const recovered = useRef(false);
  const presetRef = useRef<AudioPreset>(audioPresets[0]!);
  /** Volume leveling factor for the current track (zone setting). */
  const levelFactor = useRef(1);
  /** Zone crossfade time in seconds (0: off). */
  const crossfadeSeconds = useRef(0);
  /** Zone DSP for this device (preamp, shelves, crossfeed). */
  const dspRef = useRef<ZoneDsp | null>(null);
  /** The next source change should crossfade instead of cutting. */
  const crossfadeArmed = useRef(false);
  const crossfade = useRef<Crossfade | null>(null);
  const [audioVersion, setAudioVersion] = useState(0);
  const [audioPreset, setAudioPresetState] =
    useState<AudioPresetId>("original");
  const [isPlaying, setIsPlaying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);

  const discardPreloadedAudio = useCallback(() => {
    const element = preloadedAudio.current;
    element?.pause();
    element?.removeAttribute("src");
    element?.load();
    preloadedAudio.current = null;
    preloadedSource.current = "";
  }, []);

  const outputVolume = useCallback(
    () => leveledVolume(volumeRef.current, levelFactor.current),
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
      void fade.outgoingGraph.context.close().catch(() => {});
    }
    if (audio.current) audio.current.volume = outputVolume();
  }, [outputVolume]);

  const configureAudioElement = useCallback((element: HTMLAudioElement) => {
    element.crossOrigin = "anonymous";
    element.volume = leveledVolume(volumeRef.current, levelFactor.current);
    element.muted = mutedRef.current;
    setPitchPreservation(element, presetRef.current.preservePitch);
    element.defaultPlaybackRate = presetRef.current.rate;
    element.playbackRate = presetRef.current.rate;
  }, []);

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
    const Constructor =
      window.AudioContext ||
      (window as typeof window & { webkitAudioContext?: typeof AudioContext })
        .webkitAudioContext;
    if (!Constructor) return null;
    try {
      const context = new Constructor();
      const mediaSource = context.createMediaElementSource(element);
      const dryGain = context.createGain();
      const wetGain = context.createGain();
      const convolver = context.createConvolver();
      convolver.buffer = buildReverbImpulse(context);
      const dsp = createDspChain(context);
      mediaSource.connect(dryGain);
      convolver.connect(wetGain);
      dryGain.connect(dsp.input);
      wetGain.connect(dsp.input);
      dsp.output.connect(context.destination);
      dryGain.gain.value = 1;
      wetGain.gain.value = 0;
      dsp.apply(dspRef.current);
      graph.current = {
        convolver,
        context,
        dryGain,
        dsp,
        source: mediaSource,
        wetConnected: false,
        wetGain,
      };
    } catch (cause) {
      console.error("Unable to initialize slowed reverb audio graph", cause);
      return null;
    }
    return graph.current;
  }, [recoverAudioElement]);

  const applyPreset = useCallback(
    (preset: AudioPreset) => {
      const element = audio.current;
      if (!element) return;
      setPitchPreservation(element, preset.preservePitch);
      element.defaultPlaybackRate = preset.rate;
      element.playbackRate = preset.rate;
      const needsGraph = preset.wet > 0 || dspParameters(dspRef.current).active;
      const audioGraph =
        graph.current ?? (needsGraph ? ensureAudioGraph() : null);
      if (!audioGraph) return;
      audioGraph.dsp?.apply(dspRef.current);
      if (preset.wet > 0 && !audioGraph.wetConnected) {
        audioGraph.source.connect(audioGraph.convolver);
        audioGraph.wetConnected = true;
      } else if (preset.wet <= 0 && audioGraph.wetConnected) {
        try {
          audioGraph.source.disconnect(audioGraph.convolver);
        } catch {}
        audioGraph.wetConnected = false;
      }
      const now = audioGraph.context.currentTime;
      audioGraph.wetGain.gain.setTargetAtTime(preset.wet, now, 0.025);
      audioGraph.dryGain.gain.setTargetAtTime(preset.dry, now, 0.025);
    },
    [ensureAudioGraph],
  );

  const setAudioSource = useCallback(
    (next: string) => {
      playbackAttempt.current += 1;
      setIsPlaying(false);
      setError(null);
      source.current = next;
      if (!audio.current) return;

      const armed = crossfadeArmed.current;
      crossfadeArmed.current = false;
      // Any running fade ends before the source changes again.
      finishCrossfade();
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
        const remaining =
          Number.isFinite(previous.duration) && previous.duration > 0
            ? previous.duration - previous.currentTime
            : 0;
        const seconds = Math.min(crossfadeSeconds.current, remaining);
        if (armed && !previous.paused && seconds > 0.2) {
          // Crossfade: the outgoing track keeps playing and fades out
          // while the new one fades in (equal power).
          prepared.volume = 0;
          const fade: Crossfade = {
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
                audio.current.volume = outputVolume() * gains.incoming;
              }
              if (progress >= 1) finishCrossfade();
            }, CROSSFADE_TICK_MS),
          };
          crossfade.current = fade;
        } else {
          previous.pause();
          previous.removeAttribute("src");
          previous.load();
          if (previousGraph) {
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
      finishCrossfade,
      outputVolume,
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

  /** Zone settings for this device: crossfade time and leveling gain. */
  const setOutputSettings = useCallback(
    ({
      crossfadeSeconds: seconds,
      levelingFactor: factor,
      dsp,
    }: {
      crossfadeSeconds: number;
      levelingFactor: number;
      dsp?: ZoneDsp | null;
    }) => {
      crossfadeSeconds.current = Math.max(0, seconds);
      levelFactor.current = Number.isFinite(factor) && factor > 0 ? factor : 1;
      if (audio.current && !crossfade.current) {
        audio.current.volume = outputVolume();
      }
      dspRef.current = dsp ?? null;
      const audioGraph =
        graph.current ??
        (dspParameters(dsp).active ? ensureAudioGraph() : null);
      audioGraph?.dsp?.apply(dspRef.current);
    },
    [ensureAudioGraph, outputVolume],
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

  const setAudioVolume = useCallback((value: number | string) => {
    const next = boundedVolume(value);
    if (next === null) return;
    volumeRef.current = next;
    const leveled = leveledVolume(next, levelFactor.current);
    if (audio.current && !crossfade.current) audio.current.volume = leveled;
    if (preloadedAudio.current) preloadedAudio.current.volume = leveled;
    setVolume(next);
    if (next > 0) {
      mutedRef.current = false;
      if (audio.current) audio.current.muted = false;
      if (preloadedAudio.current) preloadedAudio.current.muted = false;
      setMuted(false);
    }
  }, []);
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
      applyPreset(preset);
      if (preloadedAudio.current) configureAudioElement(preloadedAudio.current);
      if (graph.current?.context.state === "suspended")
        void graph.current.context.resume().catch(() => {});
    },
    [applyPreset, configureAudioElement],
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

  return {
    armCrossfade,
    audio,
    audioPreset,
    finishCrossfade,
    setOutputSettings,
    audioVersion,
    backgroundWasPlaying,
    error,
    graph,
    isPlaying,
    muted,
    playAudioSource,
    preloadAudioSource,
    resumeOnReconnect,
    setAudioPreset,
    setAudioSource,
    setError,
    setIsPlaying,
    setAudioVolume,
    slowedReverb: audioPreset !== "original",
    source,
    toggleMute,
    toggleSlowedReverb,
    volume,
  };
}
