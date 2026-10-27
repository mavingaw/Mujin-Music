"use client";

import {
  activeConvolutionFilter,
  createPlaybackQueue,
  defaultRadioSettings,
  getRadioPicks,
  getRadioSettings,
  isLiveRadioId,
  isStreamingId,
  nextRepeatMode,
  normalizeDspSettings,
  radioExhaustedNotice,
  radioMinimumUpcomingPicks,
  radioPicksExhaustedNotice,
  radioQueueHistory,
  radioSectionStart,
  radioSeedParams,
  radioSkipReasonEvent,
  shouldContinueWithRadio,
  upcomingRadioPicks,
  updateRadioSettings,
  type RadioSeed,
  type RadioSettings,
  type RadioSkipReasonId,
  type ResolvedRadioSeed,
  type DspSettings,
  type ZoneRepeatMode,
} from "@parson/music-sdk";
import type { Album, Artist, LibrarySong } from "@parson/music-sdk/types";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useSession } from "@/features/account/session-provider";
import { useRadioBanIndex } from "@/features/library/radio-bans";
import { defaultCover } from "@/lib/images/default-cover";
import getBaseURL from "@/lib/api/server-url";
import streamUrl from "@/lib/api/stream-url";
import {
  browserPlaybackCapabilities,
  expectedStreamSampleRate,
} from "./playback-capabilities";
import { isCurrentTrackGeneration, setMediaPosition } from "./player-state";
import { audioPresets } from "./audio-presets";
import {
  blankAlbum,
  blankArtist,
  blankSong,
  manualQueueItems,
  normalizeSong,
  persistedQueueItems,
  radioPickItems,
  type PlayerInput,
  type QueueInput,
  type QueueItem,
  type QueueOrigin,
} from "./player-model";
import {
  useAudioEvents,
  useAudioLifecycle,
  useTransientPlayerError,
} from "./player-effects";
import { useAudioEngine } from "./use-audio-engine";
import {
  elementLeveling,
  playsInAlbumContext,
  useVolumeLevelingMode,
} from "./volume-leveling";
import type { Player } from "./player-api";
import {
  outputDeviceLabel,
  supportsOutputSelection,
  useAudioOutputs,
  useDspSettings,
  useOutputDeviceChoice,
} from "./dsp-store";
import { usePlaybackTelemetry } from "./player-telemetry";
import { useConvolutionFilters } from "./convolution-section";
import { storeQueue, storeQueueSnapshot } from "./player-queue-storage";
import { useQueueRestore } from "./use-queue-restore";
import {
  shouldAdvancePastFailedTrack,
  shouldRetryWithCompatibilityStream,
} from "./playback-recovery";
import { shouldStartCrossfade } from "./crossfade";
import { syncQueuePosition } from "./queue-position-sync";
import {
  activeQueueIndexAfterMove,
  moveQueueItem,
} from "./player-queue-reorder";
import {
  appendRadioPicks,
  markRadioPickLiked,
  withoutBannedRadioPicks,
  removeUpcomingItem,
  withoutRadioSection,
} from "./radio-queue";
import {
  clearUpcomingItems,
  insertAfterCurrent,
  removeUpcomingItem as removeUpcomingQueueItem,
  shuffleUpcomingItems,
  unshuffleUpcomingItems,
} from "./player-queue-edit";

const radioSettingsKey = ["radio-settings"] as const;
const radioContinuationItems = 25;

export type PendingRadioSkip = { song: LibrarySong; cover: string };
const cover = (path?: string) =>
  !path
    ? defaultCover
    : /^(https:\/\/|data:image\/)/i.test(path) ||
        // Live station logos are absolute URLs on this server.
        path.startsWith(`${getBaseURL()}/api/`)
      ? path
      : `${getBaseURL()}/media/images/${encodeURIComponent(path)}`;

export function usePlayerController() {
  const {
    activeSinkId,
    armCrossfade,
    audio,
    audioPreset,
    audioVersion,
    finishCrossfade,
    setOutputSettings,
    backgroundWasPlaying,
    error,
    graph,
    deviceSampleRate,
    graphSampleRate,
    isPlaying,
    loadedImpulse,
    muted,
    playAudioSource,
    preloadAudioSource,
    reapplyOutputDevice,
    resumeOnReconnect,
    setAudioPreset,
    setDspSettings,
    setOutputDevice,
    setAudioSource,
    setAudioVolume,
    setError,
    setIsPlaying,
    setLevelingGain,
    slowedReverb,
    source,
    stream,
    toggleMute,
    toggleSlowedReverb,
    volume,
  } = useAudioEngine();
  const index = useRef(0);
  const queueRef = useRef<QueueItem[]>([]);
  const activeSong = useRef<LibrarySong>(blankSong());
  const currentOrigin = useRef<QueueOrigin>("manual");
  const persistedQueue = useRef<{ id: string; revision: number } | null>(null);
  const { session } = useSession();
  const [song, setSongState] = useState<LibrarySong>(blankSong);
  const [album, setAlbumState] = useState<Album>(blankAlbum);
  const [artist, setArtistState] = useState<Artist>(blankArtist);
  const [imageSrc, setImageSrc] = useState(defaultCover);
  const [queue, setQueueState] = useState<QueueItem[]>([]);
  const playbackGeneration = useRef(0);
  const radioRequest = useRef(0);
  const compatibilityRetry = useRef(false);
  // Bitrate of the stream actually loaded (0 = original file).
  const [streamBitrate, setStreamBitrate] = useState(0);
  const [volumeLevelingMode, setVolumeLevelingMode] = useVolumeLevelingMode();
  const [outputDeviceId, setOutputDeviceId] = useOutputDeviceChoice();
  const outputs = useAudioOutputs();
  const [localDsp, setLocalDsp] = useDspSettings(outputDeviceId);
  // This device's zone keeps its chain on the server (ZonesProvider hands
  // it over); the browser's own chain applies until the zone list arrives.
  const [zoneDsp, setZoneDspState] = useState<DspSettings | null>(null);
  const setZoneDsp = useCallback(
    (settings: DspSettings | null) =>
      setZoneDspState(settings ? normalizeDspSettings(settings) : null),
    [],
  );
  const dsp = zoneDsp ?? localDsp;
  const convolutionFilterId = activeConvolutionFilter(dsp);
  const convolutionFilters = useConvolutionFilters(
    Boolean(convolutionFilterId),
  );
  const setDsp = setLocalDsp;
  useEffect(() => {
    void setOutputDevice(outputDeviceId);
  }, [outputDeviceId, outputs.devices, setOutputDevice]);
  useEffect(() => {
    setDspSettings(dsp);
  }, [dsp, setDspSettings]);
  const outputName = outputDeviceLabel(outputs.devices, activeSinkId);
  const queuePositionSync = useRef<Promise<void>>(Promise.resolve());
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const repeatRef = useRef<ZoneRepeatMode>("off");
  const [repeatMode, setRepeatModeState] = useState<ZoneRepeatMode>("off");
  const looping = repeatMode === "one";
  /** Unshuffled order while shuffle is on. */
  const originalOrder = useRef<QueueItem[] | null>(null);
  const [shuffle, setShuffleState] = useState(false);
  const queryClient = useQueryClient();
  const [activeIndex, setActiveIndex] = useState(0);
  const commitIndex = useCallback((next: number) => {
    index.current = next;
    setActiveIndex(next);
  }, []);
  const radioSettingsQuery = useQuery({
    queryKey: radioSettingsKey,
    queryFn: getRadioSettings,
    enabled: Boolean(session?.sub),
    staleTime: 5 * 60_000,
    retry: false,
  });
  const radioSettings = radioSettingsQuery.data ?? defaultRadioSettings;
  const radioEnabledRef = useRef(radioSettings.enabled);
  useEffect(() => {
    radioEnabledRef.current = radioSettings.enabled;
  }, [radioSettings.enabled]);
  const [radioSeed, setRadioSeedState] = useState<ResolvedRadioSeed | null>(
    null,
  );
  const radioSeedRef = useRef<ResolvedRadioSeed | null>(null);
  const setRadioSeed = useCallback((seed: ResolvedRadioSeed | null) => {
    radioSeedRef.current = seed;
    setRadioSeedState(seed);
  }, []);
  const rejectedSongs = useRef(new Set<string>());
  const radioTopUp = useRef<{ inFlight: boolean; exhaustedKey: string }>({
    inFlight: false,
    exhaustedKey: "",
  });
  const manualQueuePending = useRef(false);
  const [pendingRadioSkip, setPendingRadioSkip] =
    useState<PendingRadioSkip | null>(null);
  const { sendPlaybackEvent, sendSongEvent, telemetry } = usePlaybackTelemetry({
    activeSong,
    audio,
    currentOrigin,
    persistedQueue,
    userId: session?.sub,
  });

  const setSongCallback = useCallback(
    (
      next: PlayerInput,
      nextArtist?: Partial<Artist>,
      nextAlbum?: Partial<Album>,
    ) => {
      const normalized = normalizeSong(next, nextArtist, nextAlbum);
      const generation = ++playbackGeneration.current;
      compatibilityRetry.current = false;
      activeSong.current = normalized;
      telemetry.current = {
        started: false,
        qualified: false,
        completed: false,
        listenedSeconds: 0,
        lastPosition: 0,
        lastUpdateMs:
          typeof performance === "undefined" ? 0 : performance.now(),
        seeking: false,
      };
      setSongState(normalized);
      setArtistState(normalized.artist_object);
      setAlbumState(normalized.album_object);
      setImageSrc(cover(normalized.album_object.cover_url));
      setStreamBitrate(session?.bitrate ?? 0);
      setAudioSource(
        streamUrl(
          normalized.id,
          session?.bitrate ?? 0,
          false,
          browserPlaybackCapabilities(),
        ),
        session?.bitrate ? null : expectedStreamSampleRate(normalized.audio),
      );
      if (
        currentOrigin.current === "manual" &&
        !persistedQueue.current &&
        queueRef.current.length &&
        radioEnabledRef.current &&
        // Server queues and radio cover the local library only.
        !isStreamingId(normalized.id) &&
        !isLiveRadioId(normalized.id)
      ) {
        manualQueuePending.current = true;
        const explicitSongIds = queueRef.current
          .slice(index.current)
          .map((item) => item.song.id)
          .filter(Boolean);
        void createPlaybackQueue({
          seed_song_id: normalized.id,
          explicit_song_ids: explicitSongIds,
          exclude_song_ids: queueRef.current.map((item) => item.song.id),
          generated_items: 20,
          source: "manual_selection",
        })
          .then((created) => {
            if (
              !isCurrentTrackGeneration(
                generation,
                playbackGeneration.current,
                normalized.id,
                activeSong.current.id,
              ) ||
              !created.items.length
            ) {
              return;
            }
            const items = persistedQueueItems(created.items);
            persistedQueue.current = {
              id: created.id,
              revision: created.revision,
            };
            queueRef.current = items;
            commitIndex(0);
            setRadioSeed(created.seed ?? null);
            currentOrigin.current = items[0]?.origin ?? "manual";
            storeQueue(created.id);
            storeQueueSnapshot(session?.sub, items, 0);
            setQueueState(items);
          })
          .catch(() => {})
          .finally(() => {
            manualQueuePending.current = false;
          });
      }
    },
    [commitIndex, session?.bitrate, session?.sub, setAudioSource, setRadioSeed],
  );
  const setQueue = useCallback(
    (items: QueueInput[]) => {
      playbackGeneration.current += 1;
      const normalized = manualQueueItems(items);
      originalOrder.current = null;
      setShuffleState(false);
      queueRef.current = normalized;
      currentOrigin.current = "manual";
      persistedQueue.current = null;
      storeQueue(null);
      commitIndex(0);
      setRadioSeed(null);
      storeQueueSnapshot(session?.sub, normalized, 0);
      setQueueState(normalized);
    },
    [commitIndex, session?.sub, setRadioSeed],
  );
  const addToQueue = useCallback(
    (items: QueueInput[]) => {
      if (!items.length) {
        return;
      }
      const additions = manualQueueItems(items);
      if (originalOrder.current) {
        originalOrder.current = [...originalOrder.current, ...additions];
      }
      const next = [...queueRef.current, ...additions];
      queueRef.current = next;
      persistedQueue.current = null;
      storeQueue(null);
      storeQueueSnapshot(session?.sub, next, index.current);
      setQueueState(next);
    },
    [session?.sub],
  );
  const addNextToQueue = useCallback(
    (items: QueueInput[]) => {
      if (!items.length) {
        return;
      }
      const additions = manualQueueItems(items);
      if (originalOrder.current) {
        const currentId = queueRef.current[index.current]?.song.id;
        const at =
          originalOrder.current.findIndex(
            (item) => item.song.id === currentId,
          ) + 1;
        originalOrder.current = [
          ...originalOrder.current.slice(0, at),
          ...additions,
          ...originalOrder.current.slice(at),
        ];
      }
      const next = insertAfterCurrent(
        queueRef.current,
        index.current,
        additions,
      );
      queueRef.current = next;
      persistedQueue.current = null;
      storeQueue(null);
      storeQueueSnapshot(session?.sub, next, index.current);
      setQueueState(next);
    },
    [session?.sub],
  );
  const commitQueue = useCallback(
    (next: QueueItem[], nextIndex: number) => {
      queueRef.current = next;
      commitIndex(nextIndex);
      persistedQueue.current = null;
      storeQueue(null);
      storeQueueSnapshot(session?.sub, next, nextIndex);
      setQueueState(next);
    },
    [commitIndex, session?.sub],
  );
  const removeQueueItem = useCallback(
    (position: number) => {
      const removed = queueRef.current[position];
      const next = removeUpcomingQueueItem(
        queueRef.current,
        index.current,
        position,
      );
      if (!next || !removed) return;
      if (originalOrder.current) {
        const at = originalOrder.current.findIndex(
          (item) => item === removed || item.song.id === removed.song.id,
        );
        if (at >= 0) {
          originalOrder.current = originalOrder.current.filter(
            (_, position) => position !== at,
          );
        }
      }
      commitQueue(next, index.current);
    },
    [commitQueue],
  );
  const clearUpcoming = useCallback(() => {
    if (index.current + 1 >= queueRef.current.length) return;
    const next = clearUpcomingItems(queueRef.current, index.current);
    if (originalOrder.current) {
      const kept = new Set(next.map((item) => item.song.id));
      originalOrder.current = originalOrder.current.filter((item) =>
        kept.has(item.song.id),
      );
    }
    commitQueue(next, index.current);
  }, [commitQueue]);
  const setShuffle = useCallback(
    (enabled: boolean) => {
      if (enabled === Boolean(originalOrder.current)) {
        setShuffleState(enabled);
        return;
      }
      if (enabled) {
        originalOrder.current = queueRef.current;
        commitQueue(
          shuffleUpcomingItems(queueRef.current, index.current),
          index.current,
        );
      } else {
        const original = originalOrder.current ?? [];
        originalOrder.current = null;
        commitQueue(
          unshuffleUpcomingItems(
            queueRef.current,
            index.current,
            original,
            (item) => item.song.id,
          ),
          index.current,
        );
      }
      setShuffleState(enabled);
    },
    [commitQueue],
  );
  const reorderQueue = useCallback(
    (from: number, to: number) => {
      if (!Number.isFinite(from) || !Number.isFinite(to)) {
        return;
      }
      const sourceIndex = Math.trunc(from);
      const destinationIndex = Math.trunc(to);
      const items = queueRef.current;
      if (
        sourceIndex === destinationIndex ||
        sourceIndex < 0 ||
        destinationIndex < 0 ||
        sourceIndex >= items.length ||
        destinationIndex >= items.length
      ) {
        return;
      }

      const next = moveQueueItem(items, sourceIndex, destinationIndex);
      const nextActiveIndex = activeQueueIndexAfterMove(
        index.current,
        sourceIndex,
        destinationIndex,
      );
      queueRef.current = next;
      commitIndex(nextActiveIndex);
      currentOrigin.current =
        next[nextActiveIndex]?.origin ?? currentOrigin.current;
      persistedQueue.current = null;
      storeQueue(null);
      storeQueueSnapshot(session?.sub, next, nextActiveIndex);
      setQueueState(next);
    },
    [commitIndex, session?.sub],
  );
  const persistQueuePosition = useCallback((position: number | null) => {
    if (position === null || !persistedQueue.current) {
      return;
    }
    const queueId = persistedQueue.current.id;
    queuePositionSync.current = queuePositionSync.current
      .then(async () => {
        const active = persistedQueue.current;
        if (!active || active.id !== queueId) {
          return;
        }
        const saved = await syncQueuePosition(active, position);
        if (persistedQueue.current?.id === saved.id) {
          persistedQueue.current = saved;
        }
      })
      .catch(() => {});
  }, []);

  const playItem = useCallback(
    (nextIndex: number) => {
      const item = queueRef.current[nextIndex];
      if (!item) {
        return false;
      }
      commitIndex(nextIndex);
      currentOrigin.current = item.origin;
      storeQueueSnapshot(session?.sub, queueRef.current, nextIndex);
      persistQueuePosition(item.queuePosition);
      setSongCallback(item.song, item.artist, item.album);
      playAudioSource();
      return true;
    },
    [
      commitIndex,
      persistQueuePosition,
      playAudioSource,
      session?.sub,
      setSongCallback,
    ],
  );
  /**
   * A live stream that drops (network, station restart) reconnects with a
   * growing pause, up to six times in a row; a minute of good playback
   * resets the count.
   */
  const liveRetries = useRef({ count: 0, since: 0 });
  const reconnectLive = useCallback(() => {
    const songId = activeSong.current.id;
    if (!isLiveRadioId(songId)) return false;
    const now = Date.now();
    if (now - liveRetries.current.since > 60_000) {
      liveRetries.current = { count: 0, since: now };
    }
    if (liveRetries.current.count >= 6) {
      setError(
        "The station stopped sending audio. Try again later or pick another station.",
      );
      setIsPlaying(false);
      return true;
    }
    const attempt = ++liveRetries.current.count;
    const generation = playbackGeneration.current;
    setError(`Reconnecting to ${activeSong.current.name}…`);
    setTimeout(
      () => {
        if (
          generation !== playbackGeneration.current ||
          activeSong.current.id !== songId
        ) {
          return;
        }
        setAudioSource(
          streamUrl(songId, 0, false, browserPlaybackCapabilities()),
        );
        playAudioSource();
      },
      Math.min(1_000 * 2 ** (attempt - 1), 15_000),
    );
    return true;
  }, [playAudioSource, setAudioSource, setError, setIsPlaying]);
  const handleMediaError = useCallback(
    (code: number | undefined) => {
      if (reconnectLive()) return true;
      const activeSongId = activeSong.current.id;
      if (
        activeSongId &&
        shouldRetryWithCompatibilityStream(
          code,
          session?.bitrate ?? 0,
          compatibilityRetry.current,
        )
      ) {
        compatibilityRetry.current = true;
        setError("Direct playback failed; retrying a compatible audio stream.");
        setStreamBitrate(192);
        setAudioSource(streamUrl(activeSongId, 192));
        playAudioSource();
        return true;
      }
      const hasNext =
        queueRef.current.length > 0 &&
        index.current + 1 < queueRef.current.length;
      if (shouldAdvancePastFailedTrack(code, hasNext)) {
        setError("The unavailable track was skipped.");
        playItem(index.current + 1);
        return true;
      }
      return false;
    },
    [
      playAudioSource,
      playItem,
      reconnectLive,
      session?.bitrate,
      setAudioSource,
      setError,
    ],
  );
  const playNextSong = useCallback(() => {
    const element = audio.current;
    if (
      currentOrigin.current === "generated" &&
      element &&
      Number.isFinite(element.duration) &&
      telemetry.current.listenedSeconds < Math.min(30, element.duration * 0.3)
    ) {
      sendPlaybackEvent("early_skip", element.currentTime, element.duration);
      const skipped = activeSong.current;
      if (skipped.id) {
        setPendingRadioSkip({
          song: skipped,
          cover: cover(skipped.album_object.cover_url),
        });
      }
    }
    if (
      queueRef.current.length &&
      index.current + 1 < queueRef.current.length
    ) {
      playItem(index.current + 1);
    }
  }, [playItem, sendPlaybackEvent]);
  const playPreviousSong = useCallback(() => {
    const element = audio.current;
    if (
      currentOrigin.current === "generated" &&
      element &&
      telemetry.current.listenedSeconds < Math.min(30, element.duration * 0.3)
    ) {
      sendPlaybackEvent("early_skip", element.currentTime, element.duration);
    }
    if (queueRef.current.length) {
      playItem(Math.max(index.current - 1, 0));
    }
  }, [playItem, sendPlaybackEvent]);
  const togglePlayPause = useCallback(() => {
    const element = audio.current;
    if (!element) {
      return;
    }
    if (element.paused) {
      setIsPlaying(true);
      playAudioSource();
    } else {
      resumeOnReconnect.current = false;
      setIsPlaying(false);
      finishCrossfade();
      element.pause();
    }
  }, [finishCrossfade, playAudioSource, setIsPlaying]);
  const handleTimeChange = useCallback((value: number | string) => {
    // A live stream has no timeline to seek in.
    if (isLiveRadioId(activeSong.current.id)) return;
    const element = audio.current;
    const bounded = setMediaPosition(element, value, element?.duration);
    if (bounded === null) {
      return;
    }
    setCurrentTime(bounded);
  }, []);
  const setRepeatMode = useCallback((mode: ZoneRepeatMode) => {
    repeatRef.current = mode;
    setRepeatModeState(mode);
  }, []);
  /** Replaces a finished queue with a fresh Radio queue when Radio is on. */
  const continueWithRadio = useCallback(() => {
    const endingSongId = activeSong.current.id;
    if (
      !endingSongId ||
      // Server queues and radio cover the local library only.
      isStreamingId(endingSongId) ||
      isLiveRadioId(endingSongId) ||
      !shouldContinueWithRadio({
        enabled: radioEnabledRef.current,
        repeating: repeatRef.current !== "off",
      })
    ) {
      return;
    }
    const generation = playbackGeneration.current;
    const request = ++radioRequest.current;
    const seed: RadioSeed =
      currentOrigin.current === "generated" && radioSeedRef.current
        ? radioSeedRef.current
        : { kind: "song", id: endingSongId };
    void createPlaybackQueue({
      ...radioSeedParams(seed),
      // The queue so far: Radio prefers fresh songs and, once the library
      // is used up, brings back the ones heard longest ago.
      exclude_song_ids: radioQueueHistory(
        queueRef.current.map((item) => item.song.id),
        100,
      ),
      reject_song_ids: [...rejectedSongs.current],
      generated_items: radioContinuationItems,
      source: "radio",
    })
      .then((nextQueue) => {
        if (
          !isCurrentTrackGeneration(
            generation,
            playbackGeneration.current,
            endingSongId,
            activeSong.current.id,
          ) ||
          radioRequest.current !== request
        ) {
          return;
        }
        const items = persistedQueueItems(nextQueue.items);
        if (!items.length) {
          return;
        }
        persistedQueue.current = {
          id: nextQueue.id,
          revision: nextQueue.revision,
        };
        storeQueue(nextQueue.id);
        queueRef.current = items;
        setRadioSeed(nextQueue.seed ?? null);
        storeQueueSnapshot(session?.sub, items, 0);
        setQueueState(items);
        playItem(0);
      })
      .catch((error: unknown) => {
        // Radio only runs out when everything left is banned or thumbed
        // down: say so rather than stopping silently.
        const notice = radioExhaustedNotice(error);
        if (notice && radioRequest.current === request) setError(notice);
      });
  }, [playItem, session?.sub, setError, setRadioSeed]);
  const toggleLoop = useCallback(() => {
    setRepeatMode(nextRepeatMode(repeatRef.current));
  }, [setRepeatMode]);
  const handleEnded = useCallback(() => {
    // A live stream only ends when the connection drops.
    if (reconnectLive()) return;
    if (repeatRef.current === "one" && audio.current) {
      audio.current.currentTime = 0;
      playAudioSource();
      return;
    }
    setIsPlaying(false);
    const element = audio.current;
    if (!telemetry.current.completed) {
      const completionThreshold = (element?.duration ?? 0) * 0.85;
      if (
        completionThreshold > 0 &&
        telemetry.current.listenedSeconds >= completionThreshold
      ) {
        telemetry.current.completed = true;
        sendPlaybackEvent("completed", element?.currentTime, element?.duration);
      }
    }
    if (
      queueRef.current.length &&
      index.current + 1 < queueRef.current.length
    ) {
      playItem(index.current + 1);
      return;
    }
    if (repeatRef.current === "all" && queueRef.current.length) {
      playItem(0);
      return;
    }
    continueWithRadio();
  }, [continueWithRadio, playAudioSource, playItem, sendPlaybackEvent]);

  useEffect(() => {
    if (looping) {
      preloadAudioSource(null);
      return;
    }
    const next =
      queueRef.current[index.current + 1] ??
      (repeatMode === "all" ? queueRef.current[0] : undefined);
    preloadAudioSource(
      next
        ? streamUrl(
            next.song.id,
            session?.bitrate ?? 0,
            false,
            browserPlaybackCapabilities(),
          )
        : null,
    );
  }, [
    audioVersion,
    looping,
    preloadAudioSource,
    queue,
    repeatMode,
    session?.bitrate,
    song.id,
  ]);

  const replaceQueueLocally = useCallback(
    (items: QueueItem[], nextIndex = index.current) => {
      queueRef.current = items;
      persistedQueue.current = null;
      storeQueue(null);
      commitIndex(nextIndex);
      storeQueueSnapshot(session?.sub, items, nextIndex);
      setQueueState(items);
    },
    [commitIndex, session?.sub],
  );

  /** Appends Radio picks to the end of the queue (the Radio section). */
  const requestRadioPicks = useCallback(
    (limit: number) => {
      const topUp = radioTopUp.current;
      if (topUp.inFlight) return;
      const base = queueRef.current;
      const last = base.at(-1);
      const key = `${base.length}:${last?.song.id ?? ""}:${radioSeedRef.current?.id ?? ""}`;
      if (topUp.exhaustedKey === key) return;
      const seed: RadioSeed | null =
        radioSeedRef.current ??
        (last?.song.id ? { kind: "song", id: last.song.id } : null);
      topUp.inFlight = true;
      void getRadioPicks({
        seed,
        exclude_song_ids: radioQueueHistory(base.map((item) => item.song.id)),
        reject_song_ids: [...rejectedSongs.current],
        limit,
      })
        .then((result) => {
          const current = queueRef.current;
          if (!current.length || current[0]?.song.id !== base[0]?.song.id) {
            return;
          }
          // Played songs may come back once the library is used up; only
          // what is still to come must not be queued twice.
          const next = appendRadioPicks(
            current,
            radioPickItems(result.items),
            index.current + 1,
          );
          if (next.length === current.length) {
            topUp.exhaustedKey = key;
            const notice = radioPicksExhaustedNotice(result);
            if (notice) setError(notice);
            return;
          }
          if (!radioSeedRef.current && result.seed) {
            setRadioSeed(result.seed);
          }
          replaceQueueLocally(next);
        })
        .catch(() => {})
        .finally(() => {
          topUp.inFlight = false;
        });
    },
    [replaceQueueLocally, setError, setRadioSeed],
  );

  // Keep a few Radio picks visible at the end of the queue while Radio is on.
  useEffect(() => {
    if (
      !radioSettings.enabled ||
      looping ||
      !song.id ||
      isStreamingId(song.id) ||
      isLiveRadioId(song.id) ||
      manualQueuePending.current ||
      !queue.length
    ) {
      return;
    }
    const origins = queue.map((item) => item.origin);
    const radioUpcoming = upcomingRadioPicks(origins, activeIndex);
    const manualUpcoming = queue.length - activeIndex - 1 - radioUpcoming;
    if (radioUpcoming >= radioMinimumUpcomingPicks || manualUpcoming > 2) {
      return;
    }
    requestRadioPicks(radioMinimumUpcomingPicks * 2);
  }, [
    activeIndex,
    looping,
    queue,
    radioSettings.enabled,
    requestRadioPicks,
    song.id,
  ]);

  const saveRadioSettings = useCallback(
    (update: Partial<RadioSettings>) => {
      const previous =
        queryClient.getQueryData<RadioSettings>(radioSettingsKey) ??
        defaultRadioSettings;
      const optimistic = { ...previous, ...update };
      queryClient.setQueryData(radioSettingsKey, optimistic);
      radioEnabledRef.current = optimistic.enabled;
      void updateRadioSettings(update)
        .then((saved) => {
          queryClient.setQueryData(radioSettingsKey, saved);
          radioEnabledRef.current = saved.enabled;
        })
        .catch(() => {
          queryClient.setQueryData(radioSettingsKey, previous);
          radioEnabledRef.current = previous.enabled;
          setError("Radio settings could not be saved.");
        });
    },
    [queryClient, setError],
  );

  const setRadioEnabled = useCallback(
    (enabled: boolean) => {
      saveRadioSettings({ enabled });
      if (enabled) {
        radioTopUp.current.exhaustedKey = "";
        return;
      }
      const start = radioSectionStart(
        queueRef.current.map((item) => item.origin),
        index.current,
      );
      if (start >= 0) {
        replaceQueueLocally(withoutRadioSection(queueRef.current, start));
      }
    },
    [replaceQueueLocally, saveRadioSettings],
  );

  const setRadioComingUpNotice = useCallback(
    (visible: boolean) => saveRadioSettings({ coming_up_notice: visible }),
    [saveRadioSettings],
  );

  /** Starts a Radio station: Roon-style, it turns repeat off. */
  const startRadio = useCallback(
    async (seed: RadioSeed) => {
      const generation = ++playbackGeneration.current;
      let created: Awaited<ReturnType<typeof createPlaybackQueue>>;
      try {
        created = await createPlaybackQueue({
          ...radioSeedParams(seed),
          explicit_song_ids: seed.kind === "song" ? [seed.id] : [],
          exclude_song_ids: radioQueueHistory(
            queueRef.current.map((item) => item.song.id),
            100,
          ),
          reject_song_ids: [...rejectedSongs.current],
          generated_items: radioContinuationItems,
          source: "radio",
        });
      } catch (error) {
        const notice = radioExhaustedNotice(error);
        if (!notice) throw error;
        setError(notice);
        return null;
      }
      if (generation !== playbackGeneration.current) {
        return null;
      }
      const items = persistedQueueItems(created.items);
      if (!items.length) {
        return null;
      }
      setRepeatMode("off");
      if (!radioEnabledRef.current) {
        saveRadioSettings({ enabled: true });
      }
      radioTopUp.current.exhaustedKey = "";
      persistedQueue.current = { id: created.id, revision: created.revision };
      storeQueue(created.id);
      queueRef.current = items;
      setRadioSeed(created.seed ?? null);
      storeQueueSnapshot(session?.sub, items, 0);
      setQueueState(items);
      playItem(0);
      return created.seed?.name ?? items[0]?.song.name ?? "";
    },
    [playItem, saveRadioSettings, session?.sub, setRadioSeed, setRepeatMode],
  );

  const thumbsUp = useCallback(
    (itemIndex: number) => {
      const item = queueRef.current[itemIndex];
      if (!item?.song.id || item.liked) {
        return;
      }
      rejectedSongs.current.delete(item.song.id);
      void sendSongEvent(item.song.id, "thumbs_up");
      const next = markRadioPickLiked(queueRef.current, itemIndex);
      queueRef.current = next;
      storeQueueSnapshot(session?.sub, next, index.current);
      setQueueState(next);
    },
    [sendSongEvent, session?.sub],
  );

  /**
   * Thumbs down: the current track is skipped now; an upcoming Radio pick is
   * replaced by another. Either way the song stays out of Radio.
   */
  const thumbsDown = useCallback(
    (itemIndex: number) => {
      const item = queueRef.current[itemIndex];
      if (!item?.song.id) {
        return;
      }
      rejectedSongs.current.add(item.song.id);
      void sendSongEvent(item.song.id, "disliked");
      if (itemIndex === index.current) {
        setPendingRadioSkip(null);
        if (index.current + 1 < queueRef.current.length) {
          playItem(index.current + 1);
        } else {
          audio.current?.pause();
          setIsPlaying(false);
          continueWithRadio();
        }
        return;
      }
      const removed = removeUpcomingItem(
        queueRef.current,
        index.current,
        itemIndex,
      );
      if (!removed) {
        return;
      }
      replaceQueueLocally(removed.items, removed.activeIndex);
      if (item.origin === "generated" && radioEnabledRef.current) {
        radioTopUp.current.exhaustedKey = "";
        requestRadioPicks(1);
      }
    },
    [
      continueWithRadio,
      playItem,
      replaceQueueLocally,
      requestRadioPicks,
      sendSongEvent,
      setIsPlaying,
    ],
  );

  // Bans apply at once: banned Radio picks leave the queue, and a banned
  // Radio pick that is playing is skipped (explicit plays are left alone).
  const { index: banIndex, query: banQuery } = useRadioBanIndex();
  const bansLoaded = Boolean(banQuery.data);
  const appliedBanIndex = useRef<typeof banIndex | null>(null);
  useEffect(() => {
    if (!bansLoaded) {
      return;
    }
    const previous = appliedBanIndex.current;
    appliedBanIndex.current = banIndex;
    if (previous === banIndex || !queueRef.current.length) {
      return;
    }
    const result = withoutBannedRadioPicks(
      queueRef.current,
      index.current,
      banIndex,
    );
    if (result.removed > 0) {
      replaceQueueLocally(result.items, index.current);
      if (radioEnabledRef.current) {
        radioTopUp.current.exhaustedKey = "";
      }
    }
    // Only skip for a ban made while listening, not for one loaded at start.
    if (result.currentBanned && previous !== null) {
      setPendingRadioSkip(null);
      if (index.current + 1 < queueRef.current.length) {
        playItem(index.current + 1);
      } else {
        audio.current?.pause();
        setIsPlaying(false);
        continueWithRadio();
      }
    }
  }, [
    audio,
    banIndex,
    bansLoaded,
    continueWithRadio,
    playItem,
    replaceQueueLocally,
    setIsPlaying,
  ]);

  const answerRadioSkip = useCallback(
    (reason: RadioSkipReasonId) => {
      const skipped = pendingRadioSkip?.song.id;
      setPendingRadioSkip(null);
      if (!skipped) {
        return;
      }
      if (reason === "dislike") {
        rejectedSongs.current.add(skipped);
      }
      void sendSongEvent(skipped, radioSkipReasonEvent(reason));
    },
    [pendingRadioSkip?.song.id, sendSongEvent],
  );
  const dismissRadioSkip = useCallback(() => setPendingRadioSkip(null), []);
  const leveling = useMemo(
    () =>
      elementLeveling(
        song.loudness,
        volumeLevelingMode,
        volume,
        playsInAlbumContext(
          queue.map((item) => item.album.id || item.song.album_object.id),
          queue[index.current]?.song.id === song.id ? index.current : -1,
        ),
      ),
    [queue, song.id, song.loudness, volume, volumeLevelingMode],
  );
  useEffect(() => {
    setLevelingGain(leveling.gain);
  }, [leveling.gain, setLevelingGain]);
  // Crossfade (zone setting): start the next track a few seconds early.
  const crossfadeStartedFor = useRef("");
  const crossfadeSettings = useRef(0);
  const setZoneOutput = useCallback(
    (settings: { crossfadeSeconds: number; levelingFactor: number | null }) => {
      crossfadeSettings.current = settings.crossfadeSeconds;
      setOutputSettings(settings);
    },
    [setOutputSettings],
  );
  useEffect(() => {
    const element = audio.current;
    if (!element) return;
    const check = () => {
      const queueItems = queueRef.current;
      const hasNext =
        index.current + 1 < queueItems.length ||
        (repeatRef.current === "all" && queueItems.length > 1);
      if (
        !shouldStartCrossfade({
          seconds: crossfadeSettings.current,
          currentTime: element.currentTime,
          duration: element.duration,
          repeatOne: repeatRef.current === "one",
          hasNext,
          alreadyStarted: crossfadeStartedFor.current === source.current,
        }) ||
        element.paused
      ) {
        return;
      }
      crossfadeStartedFor.current = source.current;
      if (armCrossfade()) handleEnded();
    };
    element.addEventListener("timeupdate", check);
    return () => element.removeEventListener("timeupdate", check);
  }, [armCrossfade, audio, audioVersion, handleEnded, source]);

  useTransientPlayerError(error, setError);
  useAudioEvents({
    audio,
    audioVersion,
    currentOrigin,
    handleEnded,
    handleMediaError,
    resumeOnReconnect,
    sendPlaybackEvent,
    setCurrentTime,
    setDuration,
    setError,
    setIsPlaying,
    telemetry,
  });
  useAudioLifecycle({
    audio,
    backgroundWasPlaying,
    graph,
    play: playAudioSource,
    reapplyOutputDevice,
    resumeOnReconnect,
    source,
  });

  useQueueRestore({
    commitIndex,
    currentOrigin,
    generation: playbackGeneration,
    persistedQueue,
    queue: queueRef,
    setError,
    setQueue: setQueueState,
    setSong: setSongCallback,
    userId: session?.sub,
  });

  const getPlaybackPosition = useCallback(
    () => audio.current?.currentTime ?? 0,
    [],
  );

  const value = useMemo<Omit<Player, "currentTime">>(
    () => ({
      song,
      album,
      artist,
      imageSrc,
      queue,
      isPlaying,
      error,
      duration,
      volume,
      muted,
      slowedReverb,
      audioPreset,
      audioPresets,
      stream,
      looping,
      currentIndex: activeIndex,
      radio: {
        enabled: radioSettings.enabled,
        comingUpNotice: radioSettings.coming_up_notice,
        seed: radioSeed,
        pendingSkip: pendingRadioSkip,
        answerSkip: answerRadioSkip,
        dismissSkip: dismissRadioSkip,
        setEnabled: setRadioEnabled,
        setComingUpNotice: setRadioComingUpNotice,
        start: startRadio,
        thumbsDown,
        thumbsUp,
      },
      signal: {
        streamBitrate,
        engine: graphSampleRate ? "web-audio" : "media-element",
        contextSampleRate: graphSampleRate,
        leveling: leveling.result,
        levelingUnavailable: leveling.unavailable,
        dsp,
        convolution: convolutionFilterId
          ? {
              name:
                convolutionFilters.data?.find(
                  (filter) => filter.id === convolutionFilterId,
                )?.name ?? "Filter",
              loaded: loadedImpulse === convolutionFilterId,
            }
          : null,
        deviceSampleRate,
        outputName,
        outputChosen: Boolean(activeSinkId),
      },
      volumeLevelingMode,
      setVolumeLevelingMode,
      dsp,
      browserDsp: localDsp,
      setDsp,
      setZoneDsp,
      output: {
        devices: outputs.devices,
        selectedId: outputDeviceId,
        activeId: activeSinkId,
        label: outputName,
        labelsHidden: outputs.labelsHidden,
        selectable: supportsOutputSelection(),
        select: setOutputDeviceId,
        requestLabels: outputs.requestLabels,
      },
      repeatMode,
      shuffle,
      setSongCallback,
      setQueue,
      addNextToQueue,
      addToQueue,
      setCurrentSongIndex: (next) => {
        if (!Number.isFinite(next)) {
          return;
        }
        const bounded = Math.min(
          Math.max(Math.trunc(next), 0),
          Math.max(queueRef.current.length - 1, 0),
        );
        commitIndex(bounded);
        const item = queueRef.current[bounded];
        if (item) {
          currentOrigin.current = item.origin;
          storeQueueSnapshot(session?.sub, queueRef.current, bounded);
          persistQueuePosition(item.queuePosition);
        }
      },
      playAudioSource,
      togglePlayPause,
      playNextSong,
      playPreviousSong,
      playQueueItem: (nextIndex) => {
        playItem(nextIndex);
      },
      reorderQueue,
      removeQueueItem,
      clearUpcoming,
      setRepeatMode,
      setShuffle,
      handleTimeChange,
      getPlaybackPosition,
      setAudioVolume,
      setOutputSettings: setZoneOutput,
      toggleMute,
      toggleSlowedReverb,
      setAudioPreset,
      toggleLoop,
    }),
    [
      activeIndex,
      answerRadioSkip,
      dismissRadioSkip,
      pendingRadioSkip,
      radioSeed,
      radioSettings.coming_up_notice,
      radioSettings.enabled,
      setRadioComingUpNotice,
      setRadioEnabled,
      startRadio,
      thumbsDown,
      thumbsUp,
      commitIndex,
      album,
      artist,
      duration,
      handleTimeChange,
      getPlaybackPosition,
      imageSrc,
      isPlaying,
      error,
      graphSampleRate,
      convolutionFilterId,
      convolutionFilters.data,
      loadedImpulse,
      leveling,
      looping,
      activeSinkId,
      deviceSampleRate,
      dsp,
      outputDeviceId,
      outputName,
      outputs.devices,
      outputs.labelsHidden,
      outputs.requestLabels,
      localDsp,
      setDsp,
      setZoneDsp,
      setOutputDeviceId,
      muted,
      audioPreset,
      stream,
      slowedReverb,
      playAudioSource,
      playNextSong,
      playPreviousSong,
      playItem,
      reorderQueue,
      removeQueueItem,
      clearUpcoming,
      repeatMode,
      setRepeatMode,
      shuffle,
      setShuffle,
      persistQueuePosition,
      queue,
      session?.sub,
      setAudioVolume,
      setAudioPreset,
      setZoneOutput,
      setQueue,
      setVolumeLevelingMode,
      addNextToQueue,
      addToQueue,
      setSongCallback,
      song,
      streamBitrate,
      volumeLevelingMode,
      toggleMute,
      toggleLoop,
      toggleSlowedReverb,
      togglePlayPause,
      volume,
    ],
  );
  return { player: value, currentTime };
}
