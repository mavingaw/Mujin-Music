"use client";

import {
  createPlaybackQueue,
  defaultRadioSettings,
  getRadioPicks,
  getRadioSettings,
  radioMinimumUpcomingPicks,
  radioSectionStart,
  radioSeedParams,
  radioSkipReasonEvent,
  shouldContinueWithRadio,
  updateRadioSettings,
  upcomingRadioPicks,
  type RadioSeed,
  type RadioSettings,
  type RadioSkipReasonId,
  type ResolvedRadioSeed,
} from "@parson/music-sdk";
import type { Album, Artist, LibrarySong } from "@parson/music-sdk/types";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useSession } from "@/features/account/session-provider";
import { defaultCover } from "@/lib/images/default-cover";
import getBaseURL from "@/lib/api/server-url";
import streamUrl from "@/lib/api/stream-url";
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
import type { Player } from "./player-api";
import { usePlaybackTelemetry } from "./player-telemetry";
import { storeQueue, storeQueueSnapshot } from "./player-queue-storage";
import { useQueueRestore } from "./use-queue-restore";
import {
  shouldAdvancePastFailedTrack,
  shouldRetryWithCompatibilityStream,
} from "./playback-recovery";
import { syncQueuePosition } from "./queue-position-sync";
import {
  activeQueueIndexAfterMove,
  moveQueueItem,
} from "./player-queue-reorder";
import {
  appendRadioPicks,
  markRadioPickLiked,
  radioExclusions,
  removeUpcomingItem,
  withoutRadioSection,
} from "./radio-queue";

const radioSettingsKey = ["radio-settings"] as const;
const radioContinuationItems = 25;

export type PendingRadioSkip = { song: LibrarySong; cover: string };
const cover = (path?: string) =>
  path
    ? `${getBaseURL()}/media/images/${encodeURIComponent(path)}`
    : defaultCover;

export function usePlayerController() {
  const {
    audio,
    audioPreset,
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
    setAudioVolume,
    setError,
    setIsPlaying,
    slowedReverb,
    source,
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
  const queuePositionSync = useRef<Promise<void>>(Promise.resolve());
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const loopingRef = useRef(false);
  const [looping, setLooping] = useState(false);
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
      setAudioSource(streamUrl(normalized.id, session?.bitrate ?? 0));
      if (
        currentOrigin.current === "manual" &&
        !persistedQueue.current &&
        queueRef.current.length &&
        radioEnabledRef.current
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
      const insertionIndex = Math.min(
        index.current + 1,
        queueRef.current.length,
      );
      const next = [
        ...queueRef.current.slice(0, insertionIndex),
        ...additions,
        ...queueRef.current.slice(insertionIndex),
      ];
      queueRef.current = next;
      persistedQueue.current = null;
      storeQueue(null);
      storeQueueSnapshot(session?.sub, next, index.current);
      setQueueState(next);
    },
    [session?.sub],
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
  const handleMediaError = useCallback(
    (code: number | undefined) => {
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
    [playAudioSource, playItem, session?.bitrate, setAudioSource, setError],
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
      element.pause();
    }
  }, [playAudioSource, setIsPlaying]);
  const handleTimeChange = useCallback((value: number | string) => {
    const element = audio.current;
    const bounded = setMediaPosition(element, value, element?.duration);
    if (bounded === null) {
      return;
    }
    setCurrentTime(bounded);
  }, []);
  const toggleLoop = useCallback(() => {
    const next = !loopingRef.current;
    loopingRef.current = next;
    setLooping(next);
  }, []);
  /** Replaces a finished queue with a fresh Radio queue when Radio is on. */
  const continueWithRadio = useCallback(() => {
    const endingSongId = activeSong.current.id;
    if (
      !endingSongId ||
      !shouldContinueWithRadio({
        enabled: radioEnabledRef.current,
        repeating: loopingRef.current,
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
      exclude_song_ids: radioExclusions(
        queueRef.current,
        rejectedSongs.current,
      ).slice(-100),
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
      .catch(() => {});
  }, [playItem, session?.sub, setRadioSeed]);
  const handleEnded = useCallback(() => {
    if (loopingRef.current && audio.current) {
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
    continueWithRadio();
  }, [continueWithRadio, playAudioSource, playItem, sendPlaybackEvent]);

  useEffect(() => {
    if (looping) {
      preloadAudioSource(null);
      return;
    }
    const next = queueRef.current[index.current + 1];
    preloadAudioSource(
      next ? streamUrl(next.song.id, session?.bitrate ?? 0) : null,
    );
  }, [
    audioVersion,
    looping,
    preloadAudioSource,
    queue,
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
        exclude_song_ids: radioExclusions(base, rejectedSongs.current),
        limit,
      })
        .then((result) => {
          const current = queueRef.current;
          if (!current.length || current[0]?.song.id !== base[0]?.song.id) {
            return;
          }
          const next = appendRadioPicks(current, radioPickItems(result.items));
          if (next.length === current.length) {
            topUp.exhaustedKey = key;
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
    [replaceQueueLocally, setRadioSeed],
  );

  // Keep a few Radio picks visible at the end of the queue while Radio is on.
  useEffect(() => {
    if (
      !radioSettings.enabled ||
      looping ||
      !song.id ||
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
      const created = await createPlaybackQueue({
        ...radioSeedParams(seed),
        explicit_song_ids: seed.kind === "song" ? [seed.id] : [],
        exclude_song_ids: [...rejectedSongs.current].slice(-100),
        generated_items: radioContinuationItems,
        source: "radio",
      });
      if (generation !== playbackGeneration.current) {
        return null;
      }
      const items = persistedQueueItems(created.items);
      if (!items.length) {
        return null;
      }
      loopingRef.current = false;
      setLooping(false);
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
    [playItem, saveRadioSettings, session?.sub, setRadioSeed],
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
      handleTimeChange,
      getPlaybackPosition,
      setAudioVolume,
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
      looping,
      muted,
      audioPreset,
      slowedReverb,
      playAudioSource,
      playNextSong,
      playPreviousSong,
      playItem,
      reorderQueue,
      persistQueuePosition,
      queue,
      session?.sub,
      setAudioVolume,
      setAudioPreset,
      setQueue,
      addNextToQueue,
      addToQueue,
      setSongCallback,
      song,
      toggleMute,
      toggleLoop,
      toggleSlowedReverb,
      togglePlayPause,
      volume,
    ],
  );
  return { player: value, currentTime };
}
