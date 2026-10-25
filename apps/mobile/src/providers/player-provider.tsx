import {
  createPlaybackQueue,
  defaultRadioSettings,
  fillReverbImpulseChannel,
  getRadioPicks,
  getRadioSettings,
  radioExhaustedNotice,
  radioPicksExhaustedNotice,
  radioQueueHistory,
  radioSeedParams,
  radioSkipReasonEvent,
  shouldContinueWithRadio,
  updateRadioSettings,
  type RadioSeed,
  type RadioSettings,
  type RadioSkipReasonId,
  type ResolvedRadioSeed,
  getPlayerAudioPreset,
  playerAudioPresets,
  recordPlaybackEvent,
  reverbImpulseLength,
  type LibrarySong,
  type PlaybackEventType,
  type PlayerAudioPresetId,
} from "@parson/music-sdk";
import AsyncStorage from "@react-native-async-storage/async-storage";
import * as Haptics from "expo-haptics";
import {
  createAudioPlayer,
  requestNotificationPermissionsAsync,
  setAudioModeAsync,
  type AudioPlayer as ExpoAudioPlayer,
} from "expo-audio";
import {
  Audio,
  AudioContext,
  AudioManager,
  PlaybackNotificationManager,
  type AudioTagHandle,
  type ConvolverNode,
  type GainNode,
  type MediaElementAudioSourceNode,
} from "react-native-audio-api";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type PropsWithChildren,
  type RefObject,
  type SetStateAction,
} from "react";
import { Platform } from "react-native";

import {
  freshAuthorizationHeaders,
  imageUrl,
  lockScreenArtworkUrl,
  streamUrl,
} from "@/lib/runtime";
import { downloadedSongUri, hydrateDownloads } from "@/lib/downloads";
import { shouldRestartFinishedTrack } from "@/lib/playback-state";
import { playableQueueSongs } from "@/lib/playback-queue";
import {
  isCompleted,
  isEarlySkip,
  newPlaybackTelemetry,
  qualifiedPlayThreshold,
  updateListenedSeconds,
} from "@/lib/playback-telemetry";
import { restoredQueue, shuffledQueue } from "@/lib/shuffle-queue";
import {
  parseStoredPlayerState,
  serializePlayerState,
} from "@/lib/player-storage";
import { useSession } from "@/providers/session-provider";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  extendWithRadio,
  isCurrentRadioPick,
  needsRadioTopUp,
  radioStartIndex,
  withoutBannedRadioPicks,
} from "@/lib/radio-queue";
import { useCastOutput } from "@/hooks/use-cast-output";
import { useRadioBans } from "@/components/radio-bans";

type RepeatMode = "none" | "one" | "all";
export type AudioPreset = PlayerAudioPresetId;

type PlaybackSource = {
  artworkUrl?: string;
  autoplay: boolean;
  key: number;
  retryCount: number;
  value: { uri: string; headers?: Record<string, string> };
};

type NativeAudioGraph = {
  convolver: ConvolverNode;
  dryGain: GainNode;
  source: MediaElementAudioSourceNode | null;
  wetConnected: boolean;
  wetGain: GainNode;
};

type PlaybackControls = {
  cleanup: () => void;
  interrupt: () => boolean;
  next: () => void;
  pause: () => void;
  play: () => void;
  previous: () => void;
  seek: (seconds: number) => void;
};

export type PlayerRadio = {
  /** Continue with Radio picks once the queue runs out. */
  enabled: boolean;
  /** Show the "Coming up" notice before a Radio pick plays. */
  comingUpNotice: boolean;
  /** What the current station is built around, when known. */
  seed: ResolvedRadioSeed | null;
  /** Index where the Radio section starts in `queue`, or -1. */
  sectionStart: number;
  /** The current track is a Radio pick. */
  currentIsPick: boolean;
  liked: ReadonlySet<string>;
  /** A Radio pick skipped early, awaiting an optional reason. */
  pendingSkip: LibrarySong | null;
  answerSkip: (reason: RadioSkipReasonId) => void;
  dismissSkip: () => void;
  setEnabled: (enabled: boolean) => void;
  setComingUpNotice: (visible: boolean) => void;
  /** Starts a station; resolves to its name, or null when nothing plays. */
  start: (seed: RadioSeed) => Promise<string | null>;
  thumbsUp: (index: number) => void;
  /** Skips the current track, or replaces an upcoming Radio pick. */
  thumbsDown: (index: number) => void;
};

type PlayerContextValue = {
  radio: PlayerRadio;
  current: LibrarySong | null;
  currentIndex: number;
  error: string | null;
  isBuffering: boolean;
  isPlaying: boolean;
  queue: LibrarySong[];
  repeat: RepeatMode;
  shuffle: boolean;
  audioPreset: AudioPreset;
  audioPresetsEnabled: boolean;
  playSong: (song: LibrarySong, queue?: LibrarySong[]) => void;
  playShuffled: (songs: LibrarySong[]) => void;
  playAt: (index: number) => void;
  addNext: (song: LibrarySong) => void;
  addToQueue: (songs: LibrarySong[]) => void;
  clearUpcoming: () => void;
  removeFromQueue: (index: number) => void;
  toggle: () => void;
  next: () => void;
  previous: () => void;
  seek: (seconds: number) => void;
  cycleRepeat: () => void;
  toggleShuffle: () => void;
  setAudioPreset: (preset: AudioPreset) => void;
};

const PlayerContext = createContext<PlayerContextValue | null>(null);
const PlayerPositionContext = createContext({ currentTime: 0, duration: 0 });

function buildReverbImpulse(context: AudioContext) {
  const length = reverbImpulseLength(context.sampleRate);
  const impulse = context.createBuffer(2, length, context.sampleRate);
  for (let channel = 0; channel < impulse.numberOfChannels; channel += 1) {
    fillReverbImpulseChannel(impulse.getChannelData(channel), channel);
  }
  return impulse;
}

function disconnectSource(graph: NativeAudioGraph) {
  if (!graph.source) return;
  try {
    graph.source.disconnect();
  } catch {}
  graph.source = null;
  graph.wetConnected = false;
}

function useNativePlaybackControls(controls: PlaybackControls) {
  const controlsRef = useRef(controls);
  const interruptedWhilePlaying = useRef(false);

  useEffect(() => {
    controlsRef.current = controls;
  }, [controls]);

  useEffect(() => {
    void hydrateDownloads();
    if (Platform.OS === "android") return;
    AudioManager.setAudioSessionOptions({
      iosCategory: "playback",
      iosMode: "default",
      iosNotifyOthersOnDeactivation: true,
    });
    AudioManager.observeAudioInterruptions("gain");

    const interruption = AudioManager.addSystemEventListener(
      "interruption",
      ({ type, shouldResume }) => {
        if (type === "began") {
          interruptedWhilePlaying.current = controlsRef.current.interrupt();
          return;
        }
        if (shouldResume && interruptedWhilePlaying.current) {
          interruptedWhilePlaying.current = false;
          controlsRef.current.play();
        }
      },
    );
    const subscriptions = [
      interruption,
      PlaybackNotificationManager.addEventListener(
        "playbackNotificationPlay",
        () => controlsRef.current.play(),
      ),
      PlaybackNotificationManager.addEventListener(
        "playbackNotificationPause",
        () => controlsRef.current.pause(),
      ),
      PlaybackNotificationManager.addEventListener(
        "playbackNotificationNextTrack",
        () => controlsRef.current.next(),
      ),
      PlaybackNotificationManager.addEventListener(
        "playbackNotificationPreviousTrack",
        () => controlsRef.current.previous(),
      ),
      PlaybackNotificationManager.addEventListener(
        "playbackNotificationSeekTo",
        ({ value }) => controlsRef.current.seek(value),
      ),
    ];

    return () => {
      subscriptions.forEach((subscription) => subscription?.remove());
      controlsRef.current.cleanup();
      void PlaybackNotificationManager.hide().catch(() => {});
      void AudioManager.setAudioSessionActivity(false).catch(() => {});
    };
  }, []);
}

function usePlaybackNotification({
  current,
  currentTime,
  duration,
  isPlaying,
  rate,
}: {
  current: LibrarySong | null;
  currentTime: number;
  duration: number;
  isPlaying: boolean;
  rate: number;
}) {
  const controlsEnabled = useRef(false);
  const permissionRequested = useRef(false);
  const notificationUpdates = useRef<Promise<void>>(Promise.resolve());
  const elapsedTime = Math.floor(currentTime / 30) * 30;

  useEffect(() => {
    if (Platform.OS === "android") return;
    const artwork = current
      ? imageUrl(current.album_object?.cover_url)
      : undefined;

    // Older Android versions require serialized notification updates.
    notificationUpdates.current = notificationUpdates.current
      .catch(() => {})
      .then(async () => {
        if (!current) {
          controlsEnabled.current = false;
          await PlaybackNotificationManager.hide();
          return;
        }
        if (Platform.OS === "android" && !permissionRequested.current) {
          permissionRequested.current = true;
          await AudioManager.requestNotificationPermissions();
        }
        if (!controlsEnabled.current) {
          await PlaybackNotificationManager.enableControl("play", true);
          await PlaybackNotificationManager.enableControl("pause", true);
          await PlaybackNotificationManager.enableControl(
            "previousTrack",
            true,
          );
          await PlaybackNotificationManager.enableControl("nextTrack", true);
          await PlaybackNotificationManager.enableControl("seekTo", true);
          controlsEnabled.current = true;
        }
        await PlaybackNotificationManager.show({
          title: current.name,
          artist: current.artist,
          album: current.album_object?.name,
          artwork: artwork ? { uri: artwork } : undefined,
          duration,
          elapsedTime,
          speed: rate,
          state: isPlaying ? "playing" : "paused",
        });
      })
      .catch(() => {
        controlsEnabled.current = false;
      });
  }, [current, duration, elapsedTime, isPlaying, rate]);
}

function schedulePlaybackRetry(
  source: PlaybackSource,
  desiredPlayingRef: RefObject<boolean>,
  sourceSequenceRef: RefObject<number>,
  setPlaybackSource: Dispatch<SetStateAction<PlaybackSource | null>>,
) {
  if (
    !desiredPlayingRef.current ||
    source.retryCount >= 2 ||
    !/^https?:\/\//i.test(source.value.uri)
  ) {
    return false;
  }
  const failedKey = source.key;
  setTimeout(
    () => {
      setPlaybackSource((activeSource) =>
        activeSource?.key === failedKey && desiredPlayingRef.current
          ? {
              ...activeSource,
              key: ++sourceSequenceRef.current,
              retryCount: activeSource.retryCount + 1,
            }
          : activeSource,
      );
    },
    600 * (source.retryCount + 1),
  );
  return true;
}

function useAndroidAudioOutput({
  current,
  playbackSource,
  androidPlayerRef,
  playbackSourceRef,
  desiredPlayingRef,
  endedHandlerRef,
  sourceSequenceRef,
  setPlaybackSource,
  setCurrentTime,
  setDuration,
  setIsBuffering,
  setIsPlaying,
  setPlaybackError,
}: {
  current: LibrarySong | null;
  playbackSource: PlaybackSource | null;
  androidPlayerRef: RefObject<ExpoAudioPlayer | null>;
  playbackSourceRef: RefObject<PlaybackSource | null>;
  desiredPlayingRef: RefObject<boolean>;
  endedHandlerRef: RefObject<() => void>;
  sourceSequenceRef: RefObject<number>;
  setPlaybackSource: Dispatch<SetStateAction<PlaybackSource | null>>;
  setCurrentTime: Dispatch<SetStateAction<number>>;
  setDuration: Dispatch<SetStateAction<number>>;
  setIsBuffering: Dispatch<SetStateAction<boolean>>;
  setIsPlaying: Dispatch<SetStateAction<boolean>>;
  setPlaybackError: Dispatch<SetStateAction<string | null>>;
}) {
  useEffect(() => {
    if (Platform.OS !== "android") return;
    void setAudioModeAsync({
      interruptionMode: "doNotMix",
      playsInSilentMode: true,
      shouldPlayInBackground: true,
    }).catch((cause) =>
      console.error("Could not configure Android audio", cause),
    );

    const player = createAudioPlayer(null, {
      keepAudioSessionActive: true,
      preferredForwardBufferDuration: 15,
      updateInterval: 250,
    });
    androidPlayerRef.current = player;
    let finishedSourceKey = -1;
    const subscription = player.addListener(
      "playbackStatusUpdate",
      (status) => {
        if (Number.isFinite(status.currentTime))
          setCurrentTime(status.currentTime);
        if (Number.isFinite(status.duration) && status.duration > 0) {
          setDuration(status.duration);
        }
        setIsBuffering(
          status.isBuffering || (desiredPlayingRef.current && !status.isLoaded),
        );
        // Ignore stale native state while replacing the source.
        if (status.playing) setIsPlaying(true);
        else if (!desiredPlayingRef.current) setIsPlaying(false);

        const source = playbackSourceRef.current;
        if (status.error && source) {
          console.error("Could not load Android audio source", status.error);
          if (
            !schedulePlaybackRetry(
              source,
              desiredPlayingRef,
              sourceSequenceRef,
              setPlaybackSource,
            )
          ) {
            desiredPlayingRef.current = false;
            setIsBuffering(false);
            setIsPlaying(false);
            setPlaybackError(
              status.error.toLowerCase().includes("format") ||
                status.error.toLowerCase().includes("codec") ||
                status.error.toLowerCase().includes("decode")
                ? "This audio format isn’t supported on this device. Try another track or convert the file."
                : "Playback failed. Check your connection and try again.",
            );
          }
        }
        if (
          status.didJustFinish &&
          source &&
          finishedSourceKey !== source.key
        ) {
          finishedSourceKey = source.key;
          endedHandlerRef.current();
        }
      },
    );
    return () => {
      subscription.remove();
      player.clearLockScreenControls();
      player.remove();
      androidPlayerRef.current = null;
    };
  }, [
    androidPlayerRef,
    desiredPlayingRef,
    endedHandlerRef,
    playbackSourceRef,
    setCurrentTime,
    setDuration,
    setIsBuffering,
    setIsPlaying,
    setPlaybackSource,
    setPlaybackError,
    sourceSequenceRef,
  ]);

  useEffect(() => {
    if (Platform.OS !== "android" || !playbackSource) return;
    const player = androidPlayerRef.current;
    if (!player) return;
    setIsBuffering(true);
    player.replace(playbackSource.value);
    player.loop = false;
    if (current) {
      player.setActiveForLockScreen(true, {
        albumTitle: current.album_object?.name,
        artist: current.artist,
        artworkUrl: playbackSource.artworkUrl,
        title: current.name,
      });
    }
    if (playbackSource.autoplay && desiredPlayingRef.current) player.play();
  }, [
    androidPlayerRef,
    current,
    desiredPlayingRef,
    playbackSource,
    setIsBuffering,
  ]);
}

function BrowserAudioOutput({
  playbackSource,
  currentRate,
  preservePitch,
  audioRef,
  audioContext,
  desiredPlayingRef,
  sourceSequenceRef,
  setPlaybackSource,
  setCurrentTime,
  setIsBuffering,
  setIsPlaying,
  setPlaybackError,
  routeLoadedSource,
  handleEnded,
}: {
  playbackSource: PlaybackSource;
  currentRate: number;
  preservePitch: boolean;
  audioRef: RefObject<AudioTagHandle | null>;
  audioContext: AudioContext;
  desiredPlayingRef: RefObject<boolean>;
  sourceSequenceRef: RefObject<number>;
  setPlaybackSource: Dispatch<SetStateAction<PlaybackSource | null>>;
  setCurrentTime: Dispatch<SetStateAction<number>>;
  setIsBuffering: Dispatch<SetStateAction<boolean>>;
  setIsPlaying: Dispatch<SetStateAction<boolean>>;
  setPlaybackError: Dispatch<SetStateAction<string | null>>;
  routeLoadedSource: () => void;
  handleEnded: () => void;
}) {
  return (
    <Audio
      key={playbackSource.key}
      ref={audioRef}
      context={audioContext}
      source={playbackSource.value}
      autoPlay={Platform.OS === "web" ? playbackSource.autoplay : false}
      playbackRate={currentRate}
      preservesPitch={preservePitch}
      preload="auto"
      onLoadStart={() => setIsBuffering(true)}
      onLoad={routeLoadedSource}
      onError={(error) => {
        console.error("Could not load audio source", error);
        if (
          schedulePlaybackRetry(
            playbackSource,
            desiredPlayingRef,
            sourceSequenceRef,
            setPlaybackSource,
          )
        ) {
          setIsBuffering(true);
          return;
        }
        desiredPlayingRef.current = false;
        setIsBuffering(false);
        setIsPlaying(false);
        setPlaybackError(
          "Playback failed. Check your connection and try again.",
        );
      }}
      onPositionChange={setCurrentTime}
      onEnded={handleEnded}
      onPlay={() => {
        desiredPlayingRef.current = true;
        setIsBuffering(false);
        setIsPlaying(true);
      }}
      onPause={() => setIsPlaying(false)}
    />
  );
}

export function PlayerProvider({ children }: PropsWithChildren) {
  const session = useSession();
  const queryClient = useQueryClient();
  // Android playback is handled by expo-audio. Constructing the separate
  // WebAudio engine there starts an unused native audio graph and consumes a
  // substantial amount of memory/CPU on older devices.
  const [audioContext] = useState(() =>
    Platform.OS === "android" ? null : new AudioContext(),
  );
  const audioRef = useRef<AudioTagHandle>(null);
  const androidPlayerRef = useRef<ExpoAudioPlayer | null>(null);
  const graphRef = useRef<NativeAudioGraph | null>(null);
  const sourceSequence = useRef(0);
  const desiredPlaying = useRef(false);
  const endedHandlerRef = useRef<() => void>(() => {});
  const playbackSourceRef = useRef<PlaybackSource | null>(null);
  const notificationPermissionRequested = useRef(false);
  const radioRequest = useRef(0);
  const playbackOrigin = useRef<"generated" | "manual">("manual");
  const playbackSessionId = useRef("");
  const playbackEventSequence = useRef(0);
  const telemetry = useRef(newPlaybackTelemetry());
  const radioSettingsQuery = useQuery({
    queryKey: ["radio-settings"],
    queryFn: getRadioSettings,
    enabled: session.phase === "ready" && Boolean(session.claims?.sub),
    staleTime: 5 * 60_000,
    retry: false,
  });
  const radioSettings = radioSettingsQuery.data ?? defaultRadioSettings;
  const radioEnabledRef = useRef(radioSettings.enabled);
  const [radioIds, setRadioIdsState] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const radioIdsRef = useRef<ReadonlySet<string>>(radioIds);
  const [radioSeed, setRadioSeedState] = useState<ResolvedRadioSeed | null>(
    null,
  );
  const radioSeedRef = useRef<ResolvedRadioSeed | null>(null);
  const [likedIds, setLikedIds] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [pendingSkip, setPendingSkip] = useState<LibrarySong | null>(null);
  const rejectedSongs = useRef(new Set<string>());
  const radioTopUp = useRef({ inFlight: false, exhaustedKey: "" });

  const [queue, setQueue] = useState<LibrarySong[]>([]);
  const [currentIndex, setCurrentIndex] = useState(-1);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [isBuffering, setIsBuffering] = useState(false);
  const [isPlaying, setIsPlaying] = useState(false);
  const [playbackError, setPlaybackError] = useState<string | null>(null);
  const [repeat, setRepeat] = useState<RepeatMode>("none");
  const [shuffle, setShuffle] = useState(false);
  const originalQueue = useRef<LibrarySong[]>([]);
  const [audioPreset, setAudioPresetState] = useState<AudioPreset>("original");
  const [playbackSource, setPlaybackSource] = useState<PlaybackSource | null>(
    null,
  );
  const queueStorageReady = useRef(false);
  const queueStorageKey =
    session.instanceId && session.claims?.sub
      ? `parson.player-state.${session.instanceId}.${session.claims.sub}`
      : null;

  useEffect(() => {
    if (!queueStorageKey) {
      queueStorageReady.current = false;
      return;
    }
    let active = true;
    queueStorageReady.current = false;
    void AsyncStorage.getItem(queueStorageKey)
      .then((serialized) => {
        if (!active) return;
        const stored = parseStoredPlayerState(serialized);
        if (!stored) return;
        originalQueue.current = stored.originalQueue;
        playbackOrigin.current = stored.origin;
        setQueue(stored.queue);
        setCurrentIndex(stored.currentIndex);
        setRepeat(stored.repeat);
        setShuffle(stored.shuffle);
        const restored = stored.queue[stored.currentIndex];
        setDuration(Math.max(0, restored?.duration || 0));
      })
      .finally(() => {
        if (active) queueStorageReady.current = true;
      });
    return () => {
      active = false;
    };
  }, [queueStorageKey]);

  useEffect(() => {
    if (!queueStorageKey || !queueStorageReady.current) return;
    const write = setTimeout(() => {
      void AsyncStorage.setItem(
        queueStorageKey,
        serializePlayerState({
          currentIndex,
          originalQueue: originalQueue.current,
          origin: playbackOrigin.current,
          queue,
          repeat,
          shuffle,
        }),
      ).catch(() => {});
    }, 250);
    return () => clearTimeout(write);
  }, [currentIndex, queue, queueStorageKey, repeat, shuffle]);

  useEffect(() => {
    if (session.claims || session.phase === "offline") return;
    const reset = setTimeout(() => {
      sourceSequence.current += 1;
      desiredPlaying.current = false;
      audioRef.current?.pause();
      const androidPlayer = androidPlayerRef.current;
      if (androidPlayer) {
        androidPlayer.pause();
        androidPlayer.clearLockScreenControls();
      }
      if (graphRef.current) disconnectSource(graphRef.current);
      setPlaybackSource(null);
      setQueue([]);
      originalQueue.current = [];
      setShuffle(false);
      setCurrentIndex(-1);
      setCurrentTime(0);
      setDuration(0);
      setIsBuffering(false);
      setIsPlaying(false);
      setPlaybackError(null);
    }, 0);
    return () => clearTimeout(reset);
  }, [session.claims, session.phase]);

  const current = queue[currentIndex] ?? null;
  const queueRef = useRef(queue);
  const currentIndexRef = useRef(currentIndex);
  useEffect(() => {
    queueRef.current = queue;
  }, [queue]);
  useEffect(() => {
    currentIndexRef.current = currentIndex;
  }, [currentIndex]);
  useEffect(() => {
    radioEnabledRef.current = radioSettings.enabled;
  }, [radioSettings.enabled]);
  const setRadioIds = useCallback((ids: ReadonlySet<string>) => {
    radioIdsRef.current = ids;
    setRadioIdsState(ids);
  }, []);
  const addRadioIds = useCallback(
    (songs: LibrarySong[]) => {
      if (!songs.length) return;
      setRadioIds(
        new Set([...radioIdsRef.current, ...songs.map((song) => song.id)]),
      );
    },
    [setRadioIds],
  );
  const setRadioSeed = useCallback((seed: ResolvedRadioSeed | null) => {
    radioSeedRef.current = seed;
    setRadioSeedState(seed);
  }, []);
  const resetRadio = useCallback(() => {
    setRadioIds(new Set());
    setRadioSeed(null);
    radioTopUp.current.exhaustedKey = "";
  }, [setRadioIds, setRadioSeed]);
  const originFor = useCallback(
    (song: LibrarySong | undefined): "generated" | "manual" =>
      song && radioIdsRef.current.has(song.id) ? "generated" : "manual",
    [],
  );
  const preset = getPlayerAudioPreset(audioPreset);
  const audioPresetsEnabled = Platform.OS !== "android";
  const effectivePreset = audioPresetsEnabled
    ? preset
    : getPlayerAudioPreset("original");

  const sendPlaybackEvent = useCallback(
    (eventType: PlaybackEventType) => {
      if (!current?.id || !session.claims?.sub || session.phase === "offline")
        return;
      if (!playbackSessionId.current) {
        playbackSessionId.current = `mobile-${Date.now()}-${Math.random().toString(36).slice(2)}`;
      }
      playbackEventSequence.current += 1;
      void recordPlaybackEvent({
        duration_seconds: duration || current.duration,
        event_key: `${playbackSessionId.current}:${current.id}:${eventType}:${playbackEventSequence.current}`,
        event_type: eventType,
        position_seconds: currentTime,
        session_id: playbackSessionId.current,
        song_id: current.id,
        source: playbackOrigin.current,
      })
        .then(() => {
          if (eventType === "play_started") return;
          void Promise.all([
            queryClient.invalidateQueries({
              queryKey: ["home"],
              refetchType: "none",
            }),
            queryClient.invalidateQueries({
              queryKey: ["history"],
              refetchType: "none",
            }),
          ]).catch(() => {});
        })
        .catch(() => {});
    },
    [
      current,
      currentTime,
      duration,
      queryClient,
      session.claims?.sub,
      session.phase,
    ],
  );

  /** Feedback about any song, e.g. a thumbs up on an upcoming Radio pick. */
  const sendSongEvent = useCallback(
    (songId: string, eventType: PlaybackEventType) => {
      if (!songId || !session.claims?.sub || session.phase === "offline")
        return;
      if (!playbackSessionId.current) {
        playbackSessionId.current = `mobile-${Date.now()}-${Math.random().toString(36).slice(2)}`;
      }
      playbackEventSequence.current += 1;
      void recordPlaybackEvent({
        event_key: `${playbackSessionId.current}:${songId}:${eventType}:${playbackEventSequence.current}`,
        event_type: eventType,
        session_id: playbackSessionId.current,
        song_id: songId,
        source: "generated",
      }).catch(() => {});
    },
    [session.claims?.sub, session.phase],
  );

  const ensureGraph = useCallback(() => {
    if (graphRef.current) return graphRef.current;
    if (!audioContext) return null;
    const dryGain = audioContext.createGain();
    const wetGain = audioContext.createGain();
    const convolver = audioContext.createConvolver();
    convolver.buffer = buildReverbImpulse(audioContext);
    dryGain.gain.value = 1;
    wetGain.gain.value = 0;
    dryGain.connect(audioContext.destination);
    convolver.connect(wetGain);
    wetGain.connect(audioContext.destination);
    graphRef.current = {
      convolver,
      dryGain,
      source: null,
      wetConnected: false,
      wetGain,
    };
    return graphRef.current;
  }, [audioContext]);

  const applyPreset = useCallback(
    (presetId: AudioPreset) => {
      const next = getPlayerAudioPreset(presetId);
      audioRef.current?.setPlaybackRate(next.rate);
      const graph = graphRef.current;
      if (!graph || !audioContext) return;
      const now = audioContext.currentTime;
      graph.dryGain.gain.setTargetAtTime(next.dry, now, 0.025);
      graph.wetGain.gain.setTargetAtTime(next.wet, now, 0.025);
      if (next.wet > 0 && graph.source && !graph.wetConnected) {
        graph.source.connect(graph.convolver);
        graph.wetConnected = true;
      } else if (next.wet === 0 && graph.source && graph.wetConnected) {
        try {
          graph.source.disconnect(graph.convolver);
        } catch {}
        graph.wetConnected = false;
      }
    },
    [audioContext],
  );

  const routeLoadedSource = useCallback(() => {
    setIsBuffering(false);
    const audio = audioRef.current;
    if (!audio || !audioContext || Platform.OS === "web") return;
    const playWhenRequested = () => {
      if (!desiredPlaying.current) return;
      void AudioManager.setAudioSessionActivity(true).catch(() => {});
      audio.play();
    };
    // Older Android audio stacks underrun with the effects graph.
    if (Platform.OS === "android") {
      audio.setPlaybackRate(1);
      playWhenRequested();
      return;
    }
    try {
      const graph = ensureGraph();
      if (!graph) return;
      disconnectSource(graph);
      const source = audioContext.createMediaElementSource(audio);
      source.connect(graph.dryGain);
      graph.source = source;
      applyPreset(audioPreset);
      playWhenRequested();
    } catch (cause) {
      setIsPlaying(false);
      desiredPlaying.current = false;
      console.error("Could not initialize native convolution audio", cause);
    }
  }, [applyPreset, audioContext, audioPreset, ensureGraph]);

  const sourceFor = useCallback(
    async (song: LibrarySong) => {
      const local = downloadedSongUri(song.id);
      const bitrate = session.claims?.bitrate ?? 0;
      const artworkUrl = await lockScreenArtworkUrl(
        song.album_object?.cover_url,
      ).catch(() => undefined);
      return {
        artworkUrl,
        // Native decoding expects a decoded path, not an Expo file URI.
        uri: local ? decodeURI(local) : streamUrl(song.id, bitrate),
        headers: local ? undefined : await freshAuthorizationHeaders(),
      };
    },
    [session.claims?.bitrate],
  );

  const loadAt = useCallback(
    (
      songs: LibrarySong[],
      index: number,
      autoplay = true,
      preserveOriginal = false,
      origin: "generated" | "manual" = "manual",
    ) => {
      let nextSongs = songs;
      let nextIndex = index;
      if (!preserveOriginal) {
        originalQueue.current = [...songs];
        if (shuffle && songs.length > 1) {
          const shuffled = shuffledQueue(songs, index);
          nextSongs = shuffled.queue;
          nextIndex = shuffled.currentIndex;
        }
      }
      const song = nextSongs[nextIndex];
      if (!song) return;
      radioRequest.current += 1;
      playbackOrigin.current = origin;
      telemetry.current = newPlaybackTelemetry();
      if (
        autoplay &&
        Platform.OS === "android" &&
        !notificationPermissionRequested.current
      ) {
        notificationPermissionRequested.current = true;
        void requestNotificationPermissionsAsync().catch(() => {});
      }
      setQueue(nextSongs);
      setCurrentIndex(nextIndex);
      setCurrentTime(0);
      setDuration(Math.max(0, song.duration || 0));
      setIsBuffering(true);
      setIsPlaying(false);
      setPlaybackError(null);
      desiredPlaying.current = autoplay;
      audioRef.current?.pause();
      androidPlayerRef.current?.pause();
      setPlaybackSource(null);
      const key = ++sourceSequence.current;
      void sourceFor(song)
        .then(({ artworkUrl, ...value }) => {
          if (sourceSequence.current !== key) return;
          setPlaybackSource({
            artworkUrl,
            autoplay,
            key,
            retryCount: 0,
            value,
          });
        })
        .catch((cause) => {
          if (sourceSequence.current !== key) return;
          desiredPlaying.current = false;
          setIsBuffering(false);
          setIsPlaying(false);
          setPlaybackError("Could not prepare this song for playback.");
          console.error("Could not prepare audio source", cause);
        });
    },
    [shuffle, sourceFor],
  );

  const playSong = useCallback(
    (song: LibrarySong, songs = [song]) => {
      const index = Math.max(
        0,
        songs.findIndex((item) => item.id === song.id),
      );
      // Tapping a song inside the current queue keeps its Radio section.
      if (songs !== queueRef.current) resetRadio();
      loadAt(songs, index, true, false, originFor(songs[index]));
    },
    [loadAt, originFor, resetRadio],
  );

  const playAt = useCallback(
    (index: number) =>
      loadAt(queue, index, true, true, originFor(queue[index])),
    [loadAt, originFor, queue],
  );

  const playShuffled = useCallback(
    (songs: LibrarySong[]) => {
      if (!songs.length) return;
      resetRadio();
      originalQueue.current = [...songs];
      const selected = Math.floor(Math.random() * songs.length);
      const shuffled = shuffledQueue(songs, selected);
      setShuffle(true);
      loadAt(shuffled.queue, shuffled.currentIndex, true, true);
    },
    [loadAt, resetRadio],
  );

  const addNext = useCallback(
    (song: LibrarySong) => {
      const currentId = queue[currentIndex]?.id;
      const originalIndex = originalQueue.current.findIndex(
        (item) => item.id === currentId,
      );
      const insertOriginalAt =
        originalIndex >= 0 ? originalIndex + 1 : originalQueue.current.length;
      originalQueue.current = [
        ...originalQueue.current.slice(0, insertOriginalAt),
        song,
        ...originalQueue.current.slice(insertOriginalAt),
      ];
      setQueue((items) => {
        const insert = Math.max(0, currentIndex + 1);
        return [...items.slice(0, insert), song, ...items.slice(insert)];
      });
    },
    [currentIndex, queue],
  );

  const addToQueue = useCallback((songs: LibrarySong[]) => {
    if (songs.length) {
      originalQueue.current = [...originalQueue.current, ...songs];
      setQueue((items) => [...items, ...songs]);
    }
  }, []);

  const removeFromQueue = useCallback(
    (index: number) => {
      if (index < 0 || index >= queue.length || index === currentIndex) return;
      const removed = queue[index]!;
      const occurrence = queue
        .slice(0, index + 1)
        .filter((song) => song.id === removed.id).length;
      let seen = 0;
      originalQueue.current = originalQueue.current.filter((song) => {
        if (song.id !== removed.id) return true;
        seen += 1;
        return seen !== occurrence;
      });
      setQueue((items) => items.filter((_, itemIndex) => itemIndex !== index));
      if (index < currentIndex) setCurrentIndex((value) => value - 1);
    },
    [currentIndex, queue],
  );

  const clearUpcoming = useCallback(() => {
    if (currentIndex < 0 || currentIndex >= queue.length - 1) return;
    const kept = queue.slice(0, currentIndex + 1);
    const remaining = new Map<string, number>();
    kept.forEach((song) =>
      remaining.set(song.id, (remaining.get(song.id) ?? 0) + 1),
    );
    setQueue(kept);
    originalQueue.current = originalQueue.current.filter((song) => {
      const count = remaining.get(song.id) ?? 0;
      if (!count) return false;
      remaining.set(song.id, count - 1);
      return true;
    });
  }, [currentIndex, queue]);

  const play = useCallback(() => {
    if (!current) return;
    if (!playbackSource) {
      loadAt(queue, currentIndex, true, true);
      return;
    }
    if (playbackError) {
      loadAt(queue, currentIndex, true, true);
      return;
    }
    if (shouldRestartFinishedTrack(currentTime, duration)) {
      if (Platform.OS === "android") {
        void androidPlayerRef.current?.seekTo(0);
      } else {
        audioRef.current?.seekToTime(0);
      }
      setCurrentTime(0);
    }
    desiredPlaying.current = true;
    setIsPlaying(true);
    if (Platform.OS === "android") {
      androidPlayerRef.current?.play();
      return;
    }
    if (isBuffering) return;
    void AudioManager.setAudioSessionActivity(true).catch(() => {});
    audioRef.current?.play();
  }, [
    current,
    currentIndex,
    currentTime,
    duration,
    isBuffering,
    loadAt,
    playbackSource,
    playbackError,
    queue,
  ]);

  const pause = useCallback(() => {
    desiredPlaying.current = false;
    if (Platform.OS === "android") {
      androidPlayerRef.current?.pause();
      setIsPlaying(false);
      return;
    }
    audioRef.current?.pause();
    setIsPlaying(false);
  }, []);

  const interrupt = useCallback(() => {
    const wasPlaying = desiredPlaying.current;
    if (Platform.OS === "android") {
      androidPlayerRef.current?.pause();
      setIsPlaying(false);
      return wasPlaying;
    }
    audioRef.current?.pause();
    setIsPlaying(false);
    return wasPlaying;
  }, []);

  const cast = useCastOutput({
    currentIndex,
    pauseLocal: pause,
    queue,
    repeat,
  });

  useEffect(() => {
    if (!cast.casting || Platform.OS !== "android") return;
    androidPlayerRef.current?.clearLockScreenControls();
  }, [cast.casting]);

  const next = useCallback(() => {
    if (!queue.length) return;
    const nextIndex = currentIndex + 1;
    if (
      playbackOrigin.current === "generated" &&
      isEarlySkip(telemetry.current.listenedSeconds, duration)
    ) {
      sendPlaybackEvent("early_skip");
      if (current) setPendingSkip(current);
    }
    if (nextIndex < queue.length)
      loadAt(queue, nextIndex, true, true, originFor(queue[nextIndex]));
    else if (repeat === "all")
      loadAt(queue, 0, true, true, originFor(queue[0]));
    else {
      desiredPlaying.current = false;
      setIsPlaying(false);
    }
  }, [
    current,
    currentIndex,
    duration,
    loadAt,
    originFor,
    queue,
    repeat,
    sendPlaybackEvent,
  ]);

  const seek = useCallback(
    (seconds: number) => {
      const target = Math.max(0, Math.min(seconds, duration || seconds));
      if (Platform.OS === "android") {
        void androidPlayerRef.current?.seekTo(target);
      } else {
        audioRef.current?.seekToTime(target);
      }
      setCurrentTime(target);
    },
    [duration],
  );

  const previous = useCallback(() => {
    if (currentTime > 3) {
      seek(0);
      return;
    }
    const previousIndex = currentIndex - 1;
    if (
      playbackOrigin.current === "generated" &&
      isEarlySkip(telemetry.current.listenedSeconds, duration)
    ) {
      sendPlaybackEvent("early_skip");
    }
    if (previousIndex >= 0)
      loadAt(queue, previousIndex, true, true, originFor(queue[previousIndex]));
    else seek(0);
  }, [
    currentIndex,
    currentTime,
    duration,
    loadAt,
    originFor,
    queue,
    seek,
    sendPlaybackEvent,
  ]);

  const toggle = useCallback(() => {
    void Haptics.selectionAsync().catch(() => {});
    if (desiredPlaying.current) pause();
    else play();
  }, [pause, play]);

  const setAudioPreset = useCallback(
    (presetId: AudioPreset) => {
      if (!audioPresetsEnabled) return;
      if (presetId === audioPreset) return;
      setAudioPresetState(presetId);
      applyPreset(presetId);
    },
    [applyPreset, audioPreset, audioPresetsEnabled],
  );

  const cycleRepeat = useCallback(() => {
    setRepeat((value) =>
      value === "none" ? "all" : value === "all" ? "one" : "none",
    );
  }, []);

  const toggleShuffle = useCallback(() => {
    if (queue.length < 2) {
      setShuffle((value) => !value);
      return;
    }
    const currentSong = queue[currentIndex] ?? null;
    if (shuffle) {
      const restored = restoredQueue(originalQueue.current, currentSong);
      setQueue(restored.queue);
      setCurrentIndex(restored.currentIndex);
      setShuffle(false);
      return;
    }
    if (!originalQueue.current.length) originalQueue.current = [...queue];
    const shuffled = shuffledQueue(queue, currentIndex);
    setQueue(shuffled.queue);
    setCurrentIndex(shuffled.currentIndex);
    setShuffle(true);
  }, [currentIndex, queue, shuffle]);

  /** Extends a finished queue with a Radio station when Radio is on. */
  const continueWithRadio = useCallback(() => {
    desiredPlaying.current = false;
    const endingSongId = queue[currentIndex]?.id;
    if (
      !endingSongId ||
      session.phase !== "ready" ||
      !shouldContinueWithRadio({
        enabled: radioEnabledRef.current,
        repeating: repeat !== "none",
      })
    )
      return;
    const request = ++radioRequest.current;
    const seed: RadioSeed =
      radioIdsRef.current.has(endingSongId) && radioSeedRef.current
        ? radioSeedRef.current
        : { kind: "song", id: endingSongId };
    const endingQueue = queue;
    const endingIndex = currentIndex;
    void createPlaybackQueue({
      ...radioSeedParams(seed),
      // The queue so far: Radio prefers fresh songs and, once the library
      // is used up, brings back the ones heard longest ago.
      exclude_song_ids: radioQueueHistory(
        endingQueue.map((song) => song.id),
        100,
      ),
      reject_song_ids: [...rejectedSongs.current],
      generated_items: 20,
      source: "radio",
    })
      .then((generated) => {
        if (request !== radioRequest.current) return;
        const songs = playableQueueSongs(generated.items);
        const extended = extendWithRadio(endingQueue, endingIndex, songs);
        if (extended.nextIndex >= extended.queue.length) return;
        addRadioIds(songs);
        if (generated.seed) setRadioSeed(generated.seed);
        originalQueue.current = shuffle
          ? [...originalQueue.current, ...songs]
          : [...extended.queue];
        loadAt(extended.queue, extended.nextIndex, true, true, "generated");
      })
      .catch((error: unknown) => {
        // Radio only runs out when everything left is banned or thumbed
        // down: say so rather than stopping silently.
        const notice = radioExhaustedNotice(error);
        if (notice && request === radioRequest.current) {
          setPlaybackError(notice);
        }
      });
  }, [
    addRadioIds,
    currentIndex,
    loadAt,
    queue,
    repeat,
    session.phase,
    setRadioSeed,
    shuffle,
  ]);

  const handleEnded = useCallback(() => {
    setIsPlaying(false);
    if (
      !telemetry.current.completed &&
      isCompleted(telemetry.current.listenedSeconds, duration)
    ) {
      telemetry.current.completed = true;
      sendPlaybackEvent("completed");
    }
    if (repeat === "one")
      loadAt(queue, currentIndex, true, true, playbackOrigin.current);
    else if (currentIndex + 1 < queue.length)
      loadAt(
        queue,
        currentIndex + 1,
        true,
        true,
        originFor(queue[currentIndex + 1]),
      );
    else if (repeat === "all")
      loadAt(queue, 0, true, true, originFor(queue[0]));
    else continueWithRadio();
  }, [
    continueWithRadio,
    currentIndex,
    duration,
    loadAt,
    originFor,
    queue,
    repeat,
    sendPlaybackEvent,
  ]);

  /** Appends Radio picks to the Radio section at the end of the queue. */
  const requestRadioPicks = useCallback(
    (limit: number) => {
      const topUp = radioTopUp.current;
      const base = queueRef.current;
      const last = base.at(-1);
      const key = `${base.length}:${last?.id ?? ""}:${radioSeedRef.current?.id ?? ""}`;
      if (topUp.inFlight || topUp.exhaustedKey === key) return;
      const seed: RadioSeed | null =
        radioSeedRef.current ?? (last ? { kind: "song", id: last.id } : null);
      topUp.inFlight = true;
      void getRadioPicks({
        seed,
        exclude_song_ids: radioQueueHistory(base.map((song) => song.id)),
        reject_song_ids: [...rejectedSongs.current],
        limit,
      })
        .then((result) => {
          const latest = queueRef.current;
          if (!latest.length || latest[0]?.id !== base[0]?.id) return;
          // Played songs may come back once the library is used up; only
          // what is still to come must not be queued twice.
          const known = new Set(
            latest
              .slice(Math.max(0, currentIndexRef.current + 1))
              .map((song) => song.id),
          );
          const songs = playableQueueSongs(result.items).filter(
            (song) => !known.has(song.id),
          );
          if (!songs.length) {
            topUp.exhaustedKey = key;
            const notice = radioPicksExhaustedNotice(result);
            if (notice) setPlaybackError(notice);
            return;
          }
          addRadioIds(songs);
          if (!radioSeedRef.current && result.seed) setRadioSeed(result.seed);
          originalQueue.current = [...originalQueue.current, ...songs];
          setQueue((items) =>
            items[0]?.id === latest[0]?.id ? [...items, ...songs] : items,
          );
        })
        .catch(() => {})
        .finally(() => {
          topUp.inFlight = false;
        });
    },
    [addRadioIds, setRadioSeed],
  );

  const saveRadioSettings = useCallback(
    (update: Partial<RadioSettings>) => {
      const previous =
        queryClient.getQueryData<RadioSettings>(["radio-settings"]) ??
        defaultRadioSettings;
      const optimistic = { ...previous, ...update };
      queryClient.setQueryData(["radio-settings"], optimistic);
      radioEnabledRef.current = optimistic.enabled;
      void updateRadioSettings(update)
        .then((saved) => {
          queryClient.setQueryData(["radio-settings"], saved);
          radioEnabledRef.current = saved.enabled;
        })
        .catch(() => {
          queryClient.setQueryData(["radio-settings"], previous);
          radioEnabledRef.current = previous.enabled;
          setPlaybackError("Radio settings could not be saved.");
        });
    },
    [queryClient],
  );

  const setRadioEnabled = useCallback(
    (enabled: boolean) => {
      saveRadioSettings({ enabled });
      radioTopUp.current.exhaustedKey = "";
      if (enabled) return;
      const start = radioStartIndex(queue, radioIdsRef.current, currentIndex);
      if (start < 0) return;
      const removed = new Set(queue.slice(start).map((song) => song.id));
      originalQueue.current = originalQueue.current.filter(
        (song) => !removed.has(song.id),
      );
      setQueue(queue.slice(0, start));
    },
    [currentIndex, queue, saveRadioSettings],
  );

  const startRadio = useCallback(
    async (seed: RadioSeed) => {
      let created: Awaited<ReturnType<typeof createPlaybackQueue>>;
      try {
        created = await createPlaybackQueue({
          ...radioSeedParams(seed),
          explicit_song_ids: seed.kind === "song" ? [seed.id] : [],
          exclude_song_ids: radioQueueHistory(
            queueRef.current.map((song) => song.id),
            100,
          ),
          reject_song_ids: [...rejectedSongs.current],
          generated_items: 25,
          source: "radio",
        });
      } catch (error) {
        const notice = radioExhaustedNotice(error);
        if (!notice) throw error;
        setPlaybackError(notice);
        return null;
      }
      const songs = playableQueueSongs(created.items);
      if (!songs.length) return null;
      const generated = new Set(
        created.items
          .filter((item) => item.origin === "generated")
          .map((item) => item.id),
      );
      setRadioIds(generated);
      setRadioSeed(created.seed ?? null);
      radioTopUp.current.exhaustedKey = "";
      if (!radioEnabledRef.current) saveRadioSettings({ enabled: true });
      // Starting Radio turns repeat and shuffle off, as in Roon.
      setRepeat("none");
      setShuffle(false);
      originalQueue.current = [...songs];
      const first = songs[0]!;
      loadAt(
        songs,
        0,
        true,
        true,
        generated.has(first.id) ? "generated" : "manual",
      );
      return created.seed?.name ?? first.name;
    },
    [loadAt, saveRadioSettings, setRadioIds, setRadioSeed],
  );

  const thumbsUp = useCallback(
    (index: number) => {
      const song = queue[index];
      if (!song || likedIds.has(song.id)) return;
      rejectedSongs.current.delete(song.id);
      sendSongEvent(song.id, "thumbs_up");
      setLikedIds(new Set([...likedIds, song.id]));
    },
    [likedIds, queue, sendSongEvent],
  );

  const thumbsDown = useCallback(
    (index: number) => {
      const song = queue[index];
      if (!song) return;
      rejectedSongs.current.add(song.id);
      sendSongEvent(song.id, "disliked");
      if (index === currentIndex) {
        setPendingSkip(null);
        if (currentIndex + 1 < queue.length)
          loadAt(
            queue,
            currentIndex + 1,
            true,
            true,
            originFor(queue[currentIndex + 1]),
          );
        else {
          audioRef.current?.pause();
          androidPlayerRef.current?.pause();
          setIsPlaying(false);
          continueWithRadio();
        }
        return;
      }
      const wasRadioPick = radioIdsRef.current.has(song.id);
      removeFromQueue(index);
      if (wasRadioPick && radioEnabledRef.current) {
        radioTopUp.current.exhaustedKey = "";
        requestRadioPicks(1);
      }
    },
    [
      continueWithRadio,
      currentIndex,
      loadAt,
      originFor,
      queue,
      removeFromQueue,
      requestRadioPicks,
      sendSongEvent,
    ],
  );

  // Bans apply at once: banned Radio picks leave the queue, and a banned
  // Radio pick that is playing is skipped. Songs you queued yourself stay.
  const { index: banIndex, loaded: bansLoaded } = useRadioBans();
  const appliedBanIndex = useRef<typeof banIndex | null>(null);
  useEffect(() => {
    if (!bansLoaded) return;
    const previous = appliedBanIndex.current;
    appliedBanIndex.current = banIndex;
    if (previous === banIndex || !queue.length) return;
    const result = withoutBannedRadioPicks(
      queue,
      radioIdsRef.current,
      currentIndex,
      banIndex,
    );
    if (result.removedIds.length) {
      const removed = new Set(result.removedIds);
      originalQueue.current = originalQueue.current.filter(
        (song) => !removed.has(song.id),
      );
      setQueue(result.queue);
      if (radioEnabledRef.current) radioTopUp.current.exhaustedKey = "";
    }
    // Only skip for a ban made while listening, not one loaded at start.
    if (!result.currentBanned || previous === null) return;
    setPendingSkip(null);
    if (currentIndex + 1 < result.queue.length) {
      loadAt(
        result.queue,
        currentIndex + 1,
        true,
        true,
        originFor(result.queue[currentIndex + 1]),
      );
    } else {
      audioRef.current?.pause();
      androidPlayerRef.current?.pause();
      setIsPlaying(false);
      continueWithRadio();
    }
  }, [
    banIndex,
    bansLoaded,
    continueWithRadio,
    currentIndex,
    loadAt,
    originFor,
    queue,
  ]);

  const answerSkip = useCallback(
    (reason: RadioSkipReasonId) => {
      const skipped = pendingSkip?.id;
      setPendingSkip(null);
      if (!skipped) return;
      if (reason === "dislike") rejectedSongs.current.add(skipped);
      sendSongEvent(skipped, radioSkipReasonEvent(reason));
    },
    [pendingSkip?.id, sendSongEvent],
  );
  const dismissSkip = useCallback(() => setPendingSkip(null), []);

  // Keep a few Radio picks queued while Radio is on.
  useEffect(() => {
    if (
      !radioSettings.enabled ||
      repeat !== "none" ||
      cast.casting ||
      session.phase !== "ready" ||
      !needsRadioTopUp(queue, radioIds, currentIndex)
    )
      return;
    requestRadioPicks(6);
  }, [
    cast.casting,
    currentIndex,
    queue,
    radioIds,
    radioSettings.enabled,
    repeat,
    requestRadioPicks,
    session.phase,
  ]);

  useEffect(() => {
    endedHandlerRef.current = handleEnded;
  }, [handleEnded]);

  useEffect(() => {
    playbackSourceRef.current = playbackSource;
  }, [playbackSource]);

  useEffect(() => {
    if (!current?.id) return;
    const state = telemetry.current;
    updateListenedSeconds(state, currentTime, Date.now(), isPlaying);
    if (isPlaying && !state.started) {
      state.started = true;
      sendPlaybackEvent(
        playbackOrigin.current === "manual"
          ? "manual_selection"
          : "play_started",
      );
    }
    const threshold = qualifiedPlayThreshold(duration);
    if (
      !state.qualified &&
      threshold > 0 &&
      state.listenedSeconds >= threshold
    ) {
      state.qualified = true;
      sendPlaybackEvent("qualified_play");
    }
  }, [current?.id, currentTime, duration, isPlaying, sendPlaybackEvent]);

  useAndroidAudioOutput({
    current,
    playbackSource,
    androidPlayerRef,
    playbackSourceRef,
    desiredPlayingRef: desiredPlaying,
    endedHandlerRef,
    sourceSequenceRef: sourceSequence,
    setPlaybackSource,
    setCurrentTime,
    setDuration,
    setIsBuffering,
    setIsPlaying,
    setPlaybackError,
  });

  const cleanup = useCallback(() => {
    if (graphRef.current) disconnectSource(graphRef.current);
  }, []);

  const playbackControls = useMemo(
    () => ({ cleanup, interrupt, next, pause, play, previous, seek }),
    [cleanup, interrupt, next, pause, play, previous, seek],
  );
  useNativePlaybackControls(playbackControls);
  usePlaybackNotification({
    current,
    currentTime,
    duration,
    isPlaying,
    rate: effectivePreset.rate,
  });

  const value = useMemo<PlayerContextValue>(() => {
    const effectiveIndex = cast.casting ? cast.currentIndex : currentIndex;
    return {
      radio: {
        enabled: radioSettings.enabled,
        comingUpNotice: radioSettings.coming_up_notice,
        seed: radioSeed,
        sectionStart: radioStartIndex(queue, radioIds, effectiveIndex),
        currentIsPick: isCurrentRadioPick(queue, radioIds, effectiveIndex),
        liked: likedIds,
        pendingSkip,
        answerSkip,
        dismissSkip,
        setEnabled: setRadioEnabled,
        setComingUpNotice: (visible: boolean) =>
          saveRadioSettings({ coming_up_notice: visible }),
        start: startRadio,
        thumbsUp,
        thumbsDown,
      },
      current: queue[effectiveIndex] ?? current,
      currentIndex: effectiveIndex,
      error: playbackError,
      isBuffering,
      isPlaying: cast.casting ? cast.isPlaying : isPlaying,
      queue,
      repeat,
      shuffle,
      audioPreset,
      audioPresetsEnabled,
      playSong,
      playShuffled,
      playAt,
      addNext,
      addToQueue,
      clearUpcoming,
      removeFromQueue,
      toggle: cast.casting ? cast.toggle : toggle,
      next: cast.casting ? cast.next : next,
      previous: cast.casting ? cast.previous : previous,
      seek: cast.casting ? cast.seek : seek,
      cycleRepeat,
      toggleShuffle,
      setAudioPreset,
    };
  }, [
    answerSkip,
    dismissSkip,
    likedIds,
    pendingSkip,
    radioIds,
    radioSeed,
    radioSettings.coming_up_notice,
    radioSettings.enabled,
    saveRadioSettings,
    setRadioEnabled,
    startRadio,
    thumbsDown,
    thumbsUp,
    addNext,
    addToQueue,
    clearUpcoming,
    audioPreset,
    audioPresetsEnabled,
    cast.casting,
    cast.currentIndex,
    cast.isPlaying,
    cast.next,
    cast.previous,
    cast.seek,
    cast.toggle,
    current,
    currentIndex,
    cycleRepeat,
    playbackError,
    isBuffering,
    isPlaying,
    next,
    playAt,
    playSong,
    playShuffled,
    previous,
    queue,
    removeFromQueue,
    repeat,
    shuffle,
    seek,
    setAudioPreset,
    toggleShuffle,
    toggle,
  ]);
  const position = useMemo(
    () => ({
      currentTime: cast.casting ? cast.currentTime : currentTime,
      duration: cast.casting ? cast.duration : duration,
    }),
    [cast.casting, cast.currentTime, cast.duration, currentTime, duration],
  );

  return (
    <PlayerContext.Provider value={value}>
      <PlayerPositionContext.Provider value={position}>
        {children}
      </PlayerPositionContext.Provider>
      {playbackSource && Platform.OS !== "android" && audioContext ? (
        <BrowserAudioOutput
          playbackSource={playbackSource}
          currentRate={effectivePreset.rate}
          preservePitch={effectivePreset.preservePitch}
          audioRef={audioRef}
          audioContext={audioContext}
          desiredPlayingRef={desiredPlaying}
          sourceSequenceRef={sourceSequence}
          setPlaybackSource={setPlaybackSource}
          setCurrentTime={setCurrentTime}
          setIsBuffering={setIsBuffering}
          setIsPlaying={setIsPlaying}
          setPlaybackError={setPlaybackError}
          routeLoadedSource={routeLoadedSource}
          handleEnded={handleEnded}
        />
      ) : null}
    </PlayerContext.Provider>
  );
}

export function usePlayer() {
  const value = useContext(PlayerContext);
  if (!value) throw new Error("usePlayer must be used inside PlayerProvider.");
  return value;
}

export function usePlayerPosition() {
  return useContext(PlayerPositionContext);
}

export { playerAudioPresets };
