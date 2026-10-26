"use client";

import {
  addBridge,
  type BridgeStatus,
  discoverOutputs,
  removeBridge,
  discoverSonos,
  pauseAllZones,
  getSongInfos,
  getZoneEventsURL,
  getZoneLoudness,
  getZones,
  groupZones,
  levelingGainDb,
  resolveLevelBy,
  setZoneViewerDevice,
  reportClientZone,
  sendZoneCommand,
  setZoneSleepTimer,
  transferZone,
  ungroupZones,
  unseenZoneCommands,
  updateZoneSettings,
  type PendingZoneCommand,
  type Zone,
  type ZoneCommand,
  type ZoneList,
  type ZoneSettingsPatch,
  type ZoneSocketMessage,
} from "@parson/music-sdk";
import type { LibrarySong } from "@parson/music-sdk/types";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useSession } from "@/features/account/session-provider";
import { hasDesktopBridge } from "@/lib/desktop/bridge";
import { levelingFactor } from "../crossfade";
import { usePlayer } from "../player-api";
import {
  applyZoneCommand,
  heartbeatInterval,
  localZoneStatus,
  queueKey,
  type LocalZoneControls,
} from "./zone-commands";
import {
  clientZoneIdentity,
  readOrCreateDeviceId,
  readSelectedZone,
  storeSelectedZone,
} from "./zone-device";

export const zonesQueryKey = ["zones"] as const;

type ZonesContextValue = {
  zones: Zone[];
  sonos: ZoneList["sonos"] | null;
  /** UPnP renderers and this computer's audio devices. */
  outputs: ZoneList["outputs"] | null;
  /** When `zones` was fetched (for position estimates). */
  fetchedAt: number;
  loading: boolean;
  localZoneId: string | null;
  /** The zone the player bar controls; the local zone when null. */
  selectedZoneId: string | null;
  /** The selected zone when it is another device. */
  remoteZone: Zone | null;
  /** The zone picked in the picker, even before the zone list loaded. */
  requestedZoneId: string | null;
  /** True once a zone list arrived. */
  loaded: boolean;
  selectZone: (zoneId: string | null) => void;
  send: (zoneId: string, command: ZoneCommand) => Promise<void>;
  transfer: (from: string, to: string) => Promise<void>;
  group: (zoneIds: string[]) => Promise<string>;
  ungroup: (groupId: string, memberId?: string) => Promise<void>;
  setSleepTimer: (zoneId: string, minutes: number | null) => Promise<void>;
  /** Changes only the fields in `patch`. */
  updateSettings: (zoneId: string, patch: ZoneSettingsPatch) => Promise<void>;
  /** This device's zone. */
  localZone: Zone | null;
  rescanSonos: () => Promise<void>;
  /** Looks for UPnP renderers and audio devices now. */
  rescanOutputs: () => Promise<void>;
  /** Adds a Parson Bridge by address (admins). */
  addBridge: (address: string) => Promise<BridgeStatus>;
  removeBridge: (address: string) => Promise<void>;
  /** Pauses every playing zone; returns how many. */
  pauseAll: () => Promise<number>;
  busy: boolean;
  error: unknown;
};

const ZonesContext = createContext<ZonesContextValue | null>(null);

export function useZones() {
  const context = useContext(ZonesContext);
  if (!context) throw new Error("useZones must be used inside ZonesProvider");
  return context;
}

/** Optional variant for components that may render outside the provider. */
export function useOptionalZones() {
  return useContext(ZonesContext);
}

export function ZonesProvider({ children }: { children: ReactNode }) {
  const { session } = useSession();
  const signedIn = Boolean(session?.sub);
  const player = usePlayer();
  const queryClient = useQueryClient();
  const [deviceId] = useState(() => {
    if (typeof window === "undefined") return "";
    const id = readOrCreateDeviceId();
    // Zone requests name this device so its private zone stays visible.
    setZoneViewerDevice(id);
    return id;
  });
  const [localZoneId, setLocalZoneId] = useState<string | null>(null);
  const [selectedZoneId, setSelectedZoneId] = useState<string | null>(() =>
    typeof window === "undefined" ? null : readSelectedZone(),
  );
  const [socketConnected, setSocketConnected] = useState(false);

  // ----- this device as a zone ---------------------------------------
  const playerRef = useRef(player);
  useEffect(() => {
    playerRef.current = player;
  });
  const lastAppliedSeq = useRef<number | null>(null);
  const lastQueueKey = useRef<string | null>(null);
  const needsQueue = useRef(true);
  const reporting = useRef<Promise<void>>(Promise.resolve());
  const scheduledReport = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingSeek = useRef<{ songId: string; seconds: number } | null>(null);

  const controls = useMemo<LocalZoneControls>(
    () => ({
      isPlaying: () => playerRef.current.isPlaying,
      isMuted: () => playerRef.current.muted,
      play: () => {
        const current = playerRef.current;
        if (current.song.id && !current.isPlaying) current.togglePlayPause();
      },
      pause: () => {
        if (playerRef.current.isPlaying) playerRef.current.togglePlayPause();
      },
      next: () => playerRef.current.playNextSong(),
      previous: () => playerRef.current.playPreviousSong(),
      seek: (seconds) => playerRef.current.handleTimeChange(seconds),
      jump: (index) => playerRef.current.playQueueItem(index),
      setVolume: (percent) => playerRef.current.setAudioVolume(percent),
      setMuted: () => playerRef.current.toggleMute(),
      load: async (songIds, position, positionMs, play) => {
        const infos = await getSongInfos(songIds, false);
        const songs = songIds
          .map((id) => infos[id] as LibrarySong | undefined)
          .filter((song): song is LibrarySong => Boolean(song?.id));
        if (!songs.length) return;
        const index = Math.min(Math.max(position, 0), songs.length - 1);
        const items = songs.map((song) => ({
          song,
          album: song.album_object,
          artist: song.artist_object,
        }));
        const current = playerRef.current;
        current.setQueue(items);
        current.setCurrentSongIndex(index);
        const item = items[index]!;
        current.setSongCallback(item.song, item.artist, item.album);
        if (positionMs > 1_000) {
          pendingSeek.current = {
            songId: item.song.id,
            seconds: positionMs / 1000,
          };
        }
        if (play) current.playAudioSource();
      },
      songIdAt: (index) => playerRef.current.queue[index]?.song.id,
      moveQueueItem: (from, to) => playerRef.current.reorderQueue(from, to),
      removeQueueItem: (index) => playerRef.current.removeQueueItem(index),
      clearUpcoming: () => playerRef.current.clearUpcoming(),
      setRepeat: (mode) => playerRef.current.setRepeatMode(mode),
      setShuffle: (enabled) => playerRef.current.setShuffle(enabled),
    }),
    [],
  );

  // Apply a transferred position once the new track knows its duration.
  useEffect(() => {
    const seek = pendingSeek.current;
    if (!seek || player.song.id !== seek.songId || !(player.duration > 0)) {
      return;
    }
    pendingSeek.current = null;
    player.handleTimeChange(Math.min(seek.seconds, player.duration - 1));
  }, [player, player.duration, player.song.id]);

  const applyCommands = useCallback(
    async (commands: PendingZoneCommand[]) => {
      if (lastAppliedSeq.current === null) {
        // Commands queued before this page loaded are stale.
        lastAppliedSeq.current = Math.max(0, ...commands.map((c) => c.seq));
        return;
      }
      for (const pending of unseenZoneCommands(
        commands,
        lastAppliedSeq.current,
      )) {
        lastAppliedSeq.current = pending.seq;
        try {
          await applyZoneCommand(pending.command, controls);
        } catch {}
      }
    },
    [controls],
  );

  const report = useCallback(() => {
    if (!signedIn || !deviceId) return reporting.current;
    reporting.current = reporting.current
      .then(async () => {
        const current = playerRef.current;
        const songIds = current.queue.map((item) => item.song.id);
        const key = queueKey(songIds);
        const includeQueue = needsQueue.current || key !== lastQueueKey.current;
        const queueIndex = Math.max(
          0,
          songIds.indexOf(current.song.id) >= 0
            ? songIds.indexOf(current.song.id)
            : 0,
        );
        const identity = clientZoneIdentity(
          typeof navigator === "undefined" ? "" : navigator.userAgent,
          hasDesktopBridge(),
        );
        const result = await reportClientZone(deviceId, {
          ...identity,
          status: localZoneStatus({
            songId: current.song.id,
            queueSongIds: songIds,
            queueIndex,
            isPlaying: current.isPlaying,
            hasError: Boolean(current.error && !current.isPlaying),
          }),
          song_ids: includeQueue ? songIds : undefined,
          current_position: queueIndex,
          position_ms: Math.round(current.getPlaybackPosition() * 1000),
          duration_ms: Math.round(
            (Number.isFinite(current.duration) ? current.duration : 0) * 1000,
          ),
          volume: Math.round(current.volume * 100),
          muted: current.muted,
          repeat: current.repeatMode,
          shuffle: current.shuffle,
          acknowledged_seq: lastAppliedSeq.current ?? 0,
        });
        if (includeQueue) lastQueueKey.current = key;
        needsQueue.current = result.needs_queue;
        setLocalZoneId(result.zone_id);
        await applyCommands(result.commands);
      })
      .catch(() => {});
    return reporting.current;
  }, [applyCommands, deviceId, signedIn]);

  const scheduleReport = useCallback(
    (delay: number) => {
      if (scheduledReport.current) clearTimeout(scheduledReport.current);
      scheduledReport.current = setTimeout(() => {
        scheduledReport.current = null;
        void report();
      }, delay);
    },
    [report],
  );

  const status = localZoneStatus({
    songId: player.song.id,
    queueSongIds: [],
    queueIndex: 0,
    isPlaying: player.isPlaying,
    hasError: Boolean(player.error && !player.isPlaying),
  });
  const currentQueueKey = queueKey(player.queue.map((item) => item.song.id));

  // Report promptly when something a remote would notice changes.
  useEffect(() => {
    if (!signedIn) return;
    scheduleReport(350);
  }, [
    currentQueueKey,
    player.repeatMode,
    player.shuffle,
    player.muted,
    player.song.id,
    player.volume,
    scheduleReport,
    signedIn,
    status,
  ]);

  // Heartbeat.
  useEffect(() => {
    if (!signedIn) return;
    const timer = setInterval(
      () => void report(),
      heartbeatInterval(status, false),
    );
    return () => clearInterval(timer);
  }, [report, signedIn, status]);

  useEffect(
    () => () => {
      if (scheduledReport.current) clearTimeout(scheduledReport.current);
    },
    [],
  );

  // ----- zone list + events -------------------------------------------
  const query = useQuery({
    queryKey: zonesQueryKey,
    queryFn: getZones,
    enabled: signedIn,
    refetchInterval: socketConnected ? 15_000 : 5_000,
    staleTime: 1_000,
  });

  const localZoneRef = useRef(localZoneId);
  useEffect(() => {
    localZoneRef.current = localZoneId;
  }, [localZoneId]);
  useEffect(() => {
    if (!signedIn) {
      setSocketConnected(false);
      return;
    }
    let socket: WebSocket | null = null;
    let reconnect: ReturnType<typeof setTimeout> | null = null;
    let stopped = false;
    let retryDelay = 1_000;
    const connect = () => {
      if (stopped) return;
      try {
        socket = new WebSocket(getZoneEventsURL());
      } catch {
        reconnect = setTimeout(connect, retryDelay);
        retryDelay = Math.min(retryDelay * 2, 30_000);
        return;
      }
      socket.onopen = () => {
        retryDelay = 1_000;
        setSocketConnected(true);
      };
      socket.onmessage = (event) => {
        let message: ZoneSocketMessage;
        try {
          message = JSON.parse(String(event.data)) as ZoneSocketMessage;
        } catch {
          return;
        }
        if (message.type === "zones_changed") {
          void queryClient.invalidateQueries({ queryKey: zonesQueryKey });
        } else if (
          message.type === "zone_command" &&
          message.zone_id === localZoneRef.current
        ) {
          void applyCommands([
            { seq: message.seq, command: message.command },
          ]).then(() => scheduleReport(250));
        }
      };
      socket.onerror = () => socket?.close();
      socket.onclose = () => {
        setSocketConnected(false);
        if (!stopped) {
          reconnect = setTimeout(connect, retryDelay);
          retryDelay = Math.min(retryDelay * 2, 30_000);
        }
      };
    };
    connect();
    return () => {
      stopped = true;
      if (reconnect) clearTimeout(reconnect);
      socket?.close();
    };
  }, [applyCommands, queryClient, scheduleReport, signedIn]);

  // ----- actions ------------------------------------------------------
  const refresh = useCallback(
    () => queryClient.invalidateQueries({ queryKey: zonesQueryKey }),
    [queryClient],
  );
  const action = useMutation({
    mutationFn: async (run: () => Promise<unknown>) => run(),
    onSettled: () => void refresh(),
  });
  const run = action.mutateAsync;

  const selectZone = useCallback(
    (zoneId: string | null) => {
      const next = zoneId && zoneId !== localZoneId ? zoneId : null;
      setSelectedZoneId(next);
      storeSelectedZone(next);
    },
    [localZoneId],
  );

  const zones = useMemo(() => query.data?.zones ?? [], [query.data]);
  const remoteZone = useMemo(() => {
    if (!selectedZoneId || selectedZoneId === localZoneId) return null;
    return zones.find((zone) => zone.id === selectedZoneId) ?? null;
  }, [localZoneId, selectedZoneId, zones]);

  const localZone = useMemo(
    () => zones.find((zone) => zone.id === localZoneId) ?? null,
    [localZoneId, zones],
  );

  // Enforce this zone's safety limits on the local player too.
  const localMin = localZone?.volume?.min;
  const localMax = localZone?.volume?.max;
  useEffect(() => {
    const percent = player.volume * 100;
    if (localMax !== undefined && percent > localMax + 0.5) {
      player.setAudioVolume(localMax);
    } else if (
      localMin !== undefined &&
      localMin > 0 &&
      percent < localMin - 0.5
    ) {
      player.setAudioVolume(localMin);
    }
  }, [localMax, localMin, player, player.volume]);

  // Crossfade and volume leveling from this zone's settings.
  const localSettings = localZone?.settings;
  const crossfadeSeconds = localSettings?.supports.crossfade
    ? localSettings.crossfade_seconds
    : 0;
  const leveling =
    localSettings?.supports.leveling && localSettings.leveling !== "off"
      ? localSettings
      : null;
  const queueSongIds = useMemo(
    () => player.queue.map((item) => item.song.id),
    [player.queue],
  );
  const currentIndex = queueSongIds.indexOf(player.song.id);
  const loudnessIds = leveling
    ? [player.song.id, queueSongIds[currentIndex + 1]].filter(
        (id): id is string => Boolean(id),
      )
    : [];
  const loudness = useQuery({
    queryKey: ["zone-loudness", loudnessIds.join(",")],
    queryFn: () => getZoneLoudness(loudnessIds),
    enabled: signedIn && loudnessIds.length > 0,
    staleTime: Infinity,
    // Album loudness appears once the whole album is measured.
    refetchInterval: (state) => (state.state.data?.pending ? 4_000 : false),
  }).data;
  const levelingGain = useMemo(() => {
    if (!leveling || !player.song.id) return null;
    const measured = loudness?.tracks[player.song.id];
    if (!measured) return null;
    const by = resolveLevelBy(
      leveling.leveling,
      queueSongIds,
      Math.max(currentIndex, 0),
      (songId) =>
        player.queue.find((item) => item.song.id === songId)?.album.id ?? null,
    );
    const source =
      by === "album" ? (measured.album ?? measured.track) : measured.track;
    return source
      ? levelingGainDb(
          source,
          leveling.leveling_target_lufs,
          leveling.leveling_adjustment_db,
        )
      : null;
  }, [
    currentIndex,
    leveling,
    loudness,
    player.queue,
    player.song.id,
    queueSongIds,
  ]);
  // Per-zone DSP for this device's own playback.
  const dsp = localSettings?.supports.dsp ? (localSettings.dsp ?? null) : null;
  const dspKey = dsp ? JSON.stringify(dsp) : "";
  const { setOutputSettings } = player;
  useEffect(() => {
    setOutputSettings({
      crossfadeSeconds,
      // Without a zone measurement the track's ReplayGain leveling applies.
      levelingFactor:
        levelingGain === null ? null : levelingFactor(levelingGain),
      dsp: dspKey ? (JSON.parse(dspKey) as NonNullable<typeof dsp>) : null,
    });
  }, [crossfadeSeconds, dspKey, levelingGain, setOutputSettings]);

  const value = useMemo<ZonesContextValue>(
    () => ({
      zones,
      sonos: query.data?.sonos ?? null,
      outputs: query.data?.outputs ?? null,
      fetchedAt: query.dataUpdatedAt,
      loading: query.isLoading,
      localZoneId,
      selectedZoneId: remoteZone ? remoteZone.id : localZoneId,
      remoteZone,
      requestedZoneId: selectedZoneId,
      loaded: query.dataUpdatedAt > 0,
      selectZone,
      send: async (zoneId, command) => {
        await run(() => sendZoneCommand(zoneId, command));
      },
      transfer: async (from, to) => {
        // Make sure the server has this device's latest queue and position.
        if (from === localZoneId) await report();
        await run(() => transferZone(from, to));
        selectZone(to);
      },
      group: async (zoneIds) => {
        const result = (await run(() => groupZones(zoneIds))) as Awaited<
          ReturnType<typeof groupZones>
        >;
        return result.group_id;
      },
      ungroup: async (groupId, memberId) => {
        await run(() => ungroupZones(groupId, memberId));
      },
      setSleepTimer: async (zoneId, minutes) => {
        await run(() => setZoneSleepTimer(zoneId, minutes));
      },
      updateSettings: async (zoneId, patch) => {
        await run(() => updateZoneSettings(zoneId, patch));
      },
      localZone,
      rescanSonos: async () => {
        await run(() => discoverSonos());
      },
      rescanOutputs: async () => {
        await run(() => discoverOutputs());
      },
      addBridge: async (address) =>
        (await run(() => addBridge(address))) as BridgeStatus,
      removeBridge: async (address) => {
        await run(() => removeBridge(address));
      },
      pauseAll: async () => {
        const result = (await run(() => pauseAllZones())) as Awaited<
          ReturnType<typeof pauseAllZones>
        >;
        return result.paused;
      },
      busy: action.isPending,
      error: action.error ?? query.error,
    }),
    [
      action.error,
      action.isPending,
      localZone,
      localZoneId,
      query.data?.sonos,
      query.data?.outputs,
      query.dataUpdatedAt,
      query.error,
      query.isLoading,
      remoteZone,
      report,
      run,
      selectZone,
      selectedZoneId,
      zones,
    ],
  );

  return (
    <ZonesContext.Provider value={value}>{children}</ZonesContext.Provider>
  );
}
