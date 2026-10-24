import {
  buildSignalPath,
  getPlayerAudioPreset,
  getSongSignalPath,
  type LibrarySong,
  type SignalPath,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";

import { mobileClientLeg } from "@/lib/signal-path";
import { usePlayer } from "@/providers/player-provider";
import { useSession } from "@/providers/session-provider";

/** Server leg (fetched) plus this device's leg for a song. */
export function useSignalPath(song: LibrarySong | null): {
  path: SignalPath | null;
  loading: boolean;
} {
  const player = usePlayer();
  const session = useSession();
  const { signal } = player;
  const online = session.phase !== "offline";
  const bitrate = signal.casting ? 0 : signal.streamBitrate;
  const target = signal.casting ? "cast" : "direct";
  const songId = song?.id ?? "";
  const server = useQuery({
    queryKey: ["signal-path", songId, bitrate, target],
    queryFn: () => getSongSignalPath(songId, { bitrate, target }),
    enabled: Boolean(songId) && online && !signal.localFile,
    retry: 1,
    staleTime: 60_000,
  });
  const preset = getPlayerAudioPreset(
    player.audioPresetsEnabled ? player.audioPreset : "original",
  );
  const path = useMemo(
    () =>
      song
        ? buildSignalPath(
            server.data ?? null,
            mobileClientLeg(signal, preset),
            song.audio,
          )
        : null,
    [preset, server.data, signal, song],
  );
  return { path, loading: server.isLoading };
}
