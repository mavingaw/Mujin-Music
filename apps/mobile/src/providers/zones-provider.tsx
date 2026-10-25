import {
  createZoneDeviceId,
  getSongInfos,
  getZones,
  groupZones,
  reportClientZone,
  pauseAllZones,
  sendZoneCommand,
  setZoneViewerDevice,
  transferZone,
  ungroupZones,
  unseenZoneCommands,
  updateZoneSettings,
  type LibrarySong,
  type PendingZoneCommand,
  type Zone,
  type ZoneCommand,
  type ZoneSettingsPatch,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import * as Device from "expo-device";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type PropsWithChildren,
} from "react";
import { AppState, Platform } from "react-native";

import { getSecureItem, setSecureItem } from "@/lib/secure-storage";
import {
  applyMobileZoneCommand,
  mobileZoneName,
  mobileZoneStatus,
  zonePollInterval,
  zoneRepeatMode,
  ZONE_DEVICE_KEY,
  ZONE_SELECTED_KEY,
  type MobileZoneControls,
} from "@/lib/zones";
import { usePlayer, usePlayerPosition } from "@/providers/player-provider";
import { useSession } from "@/providers/session-provider";

type ZonesValue = {
  zones: Zone[];
  fetchedAt: number;
  localZoneId: string | null;
  selectedZone: Zone | null;
  select: (zoneId: string | null) => void;
  send: (zoneId: string, command: ZoneCommand) => Promise<void>;
  transfer: (from: string, to: string) => Promise<void>;
  group: (zoneIds: string[]) => Promise<void>;
  ungroup: (groupId: string, memberId?: string) => Promise<void>;
  /** Changes only the fields in `patch`. */
  updateSettings: (zoneId: string, patch: ZoneSettingsPatch) => Promise<void>;
  setSheetOpen: (open: boolean) => void;
  refresh: () => void;
  /** Pauses every playing zone (Roon's "Pause all"); returns how many. */
  pauseAll: () => Promise<number>;
};

const ZonesContext = createContext<ZonesValue | null>(null);

export function useZones() {
  const value = useContext(ZonesContext);
  if (!value) throw new Error("useZones must be used inside ZonesProvider.");
  return value;
}

export function ZonesProvider({ children }: PropsWithChildren) {
  const session = useSession();
  const ready = session.phase === "ready";
  const player = usePlayer();
  const position = usePlayerPosition();
  const queryClient = useQueryClient();
  const [deviceId, setDeviceId] = useState<string | null>(null);
  const [localZoneId, setLocalZoneId] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [sheetOpen, setSheetOpen] = useState(false);
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const playerRef = useRef(player);
  const positionRef = useRef(position);
  useEffect(() => {
    playerRef.current = player;
    positionRef.current = position;
  });
  const lastSeq = useRef<number | null>(null);
  const lastQueue = useRef<string | null>(null);
  const pendingSeek = useRef<{ songId: string; seconds: number } | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      let id = await getSecureItem(ZONE_DEVICE_KEY);
      if (!id || !/^[a-z0-9_-]{8,64}$/i.test(id)) {
        id = createZoneDeviceId();
        await setSecureItem(ZONE_DEVICE_KEY, id);
      }
      const selected = await getSecureItem(ZONE_SELECTED_KEY);
      if (!cancelled) {
        // Zone requests name this phone so its private zone stays visible.
        setZoneViewerDevice(id);
        setDeviceId(id);
        setSelectedId(selected || null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) =>
      setForeground(state === "active"),
    );
    return () => subscription.remove();
  }, []);

  const controls = useMemo<MobileZoneControls>(
    () => ({
      isPlaying: () => playerRef.current.isPlaying,
      toggle: () => playerRef.current.toggle(),
      next: () => playerRef.current.next(),
      previous: () => playerRef.current.previous(),
      seek: (seconds) => playerRef.current.seek(seconds),
      playAt: (index) => playerRef.current.playAt(index),
      load: async (songIds, index, positionMs) => {
        const infos = await getSongInfos(songIds, false);
        const songs = songIds
          .map((id) => infos[id] as LibrarySong | undefined)
          .filter((song): song is LibrarySong => Boolean(song?.album_object));
        const start = songs[Math.min(index, songs.length - 1)];
        if (!start) return;
        playerRef.current.playSong(start, songs);
        if (positionMs > 1_000) {
          pendingSeek.current = {
            songId: start.id,
            seconds: positionMs / 1000,
          };
        }
      },
      songIdAt: (index) => playerRef.current.queue[index]?.id,
      move: (from, to) => playerRef.current.moveInQueue(from, to),
      remove: (index) => playerRef.current.removeFromQueue(index),
      clearUpcoming: () => playerRef.current.clearUpcoming(),
      setRepeat: (mode) => playerRef.current.setRepeatMode(mode),
      setShuffle: (enabled) => playerRef.current.setShuffleEnabled(enabled),
    }),
    [],
  );

  useEffect(() => {
    const seek = pendingSeek.current;
    if (
      !seek ||
      player.current?.id !== seek.songId ||
      !(position.duration > 0)
    ) {
      return;
    }
    pendingSeek.current = null;
    player.seek(Math.min(seek.seconds, position.duration - 1));
  }, [player, position.duration]);

  const apply = useCallback(
    async (commands: PendingZoneCommand[]) => {
      if (lastSeq.current === null) {
        lastSeq.current = Math.max(0, ...commands.map((entry) => entry.seq));
        return;
      }
      for (const entry of unseenZoneCommands(commands, lastSeq.current)) {
        lastSeq.current = entry.seq;
        try {
          await applyMobileZoneCommand(entry.command, controls);
        } catch {}
      }
    },
    [controls],
  );

  const report = useCallback(async () => {
    if (!ready || !deviceId) return;
    const current = playerRef.current;
    const songIds = current.queue.map((song) => song.id);
    const queueKey = songIds.join(",");
    const includeQueue = queueKey !== lastQueue.current;
    try {
      const result = await reportClientZone(deviceId, {
        kind: "mobile",
        name: mobileZoneName(Device.deviceName, Device.modelName, Platform.OS),
        device: Device.modelName ?? Platform.OS,
        status: mobileZoneStatus(
          Boolean(current.current),
          current.isPlaying,
          current.isBuffering,
        ),
        song_ids: includeQueue ? songIds : undefined,
        current_position: Math.max(0, current.currentIndex),
        position_ms: Math.round(positionRef.current.currentTime * 1000),
        duration_ms: Math.round(positionRef.current.duration * 1000),
        volume: 100,
        muted: false,
        repeat: zoneRepeatMode(current.repeat),
        shuffle: current.shuffle,
        acknowledged_seq: lastSeq.current ?? 0,
      });
      if (includeQueue) lastQueue.current = queueKey;
      if (result.needs_queue) lastQueue.current = null;
      setLocalZoneId(result.zone_id);
      await apply(result.commands);
    } catch {}
  }, [apply, deviceId, ready]);

  const playing = player.isPlaying;
  const interval = zonePollInterval(sheetOpen, playing);
  useEffect(() => {
    if (!ready || !deviceId || !foreground) return;
    void report();
    const timer = setInterval(() => void report(), interval);
    return () => clearInterval(timer);
  }, [deviceId, foreground, interval, ready, report]);

  // Prompt reports on track, play-state, queue and mode changes.
  const currentId = player.current?.id;
  const queueKey = player.queue.map((song) => song.id).join(",");
  const { repeat, shuffle } = player;
  useEffect(() => {
    if (ready && deviceId) void report();
  }, [currentId, deviceId, playing, queueKey, repeat, ready, report, shuffle]);

  const query = useQuery({
    queryKey: ["zones"],
    queryFn: getZones,
    enabled: ready && foreground,
    refetchInterval: sheetOpen || selectedId ? 2_000 : 15_000,
  });
  const zones = useMemo(() => query.data?.zones ?? [], [query.data]);
  const selectedZone =
    selectedId && selectedId !== localZoneId
      ? (zones.find((zone) => zone.id === selectedId) ?? null)
      : null;

  const refresh = useCallback(
    () => void queryClient.invalidateQueries({ queryKey: ["zones"] }),
    [queryClient],
  );

  const value = useMemo<ZonesValue>(
    () => ({
      zones,
      fetchedAt: query.dataUpdatedAt,
      localZoneId,
      selectedZone,
      select: (zoneId) => {
        const next = zoneId && zoneId !== localZoneId ? zoneId : null;
        setSelectedId(next);
        void setSecureItem(ZONE_SELECTED_KEY, next ?? "");
      },
      send: async (zoneId, command) => {
        await sendZoneCommand(zoneId, command);
        refresh();
      },
      transfer: async (from, to) => {
        if (from === localZoneId) await report();
        await transferZone(from, to);
        setSelectedId(to === localZoneId ? null : to);
        refresh();
      },
      group: async (zoneIds) => {
        await groupZones(zoneIds);
        refresh();
      },
      ungroup: async (groupId, memberId) => {
        await ungroupZones(groupId, memberId);
        refresh();
      },
      updateSettings: async (zoneId, patch) => {
        await updateZoneSettings(zoneId, patch);
        refresh();
      },
      setSheetOpen,
      refresh,
      pauseAll: async () => {
        const { paused } = await pauseAllZones();
        refresh();
        return paused;
      },
    }),
    [localZoneId, query.dataUpdatedAt, refresh, report, selectedZone, zones],
  );

  return (
    <ZonesContext.Provider value={value}>{children}</ZonesContext.Provider>
  );
}
