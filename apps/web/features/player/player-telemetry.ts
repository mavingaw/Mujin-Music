"use client";

import {
  isStreamingId,
  recordPlaybackEvent,
  type PlaybackEventType,
} from "@parson/music-sdk";
import type { LibrarySong } from "@parson/music-sdk/types";
import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useRef, type MutableRefObject } from "react";
import type { PlaybackTelemetry } from "./player-effects";
import type { QueueOrigin } from "./player-model";

function createSessionId() {
  if (typeof crypto !== "undefined") {
    if (typeof crypto.randomUUID === "function") return crypto.randomUUID();

    if (typeof crypto.getRandomValues === "function") {
      const bytes = crypto.getRandomValues(new Uint8Array(16));
      return `session-${Array.from(bytes, (byte) =>
        byte.toString(16).padStart(2, "0"),
      ).join("")}`;
    }
  }

  return `session-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export function usePlaybackTelemetry({
  activeSong,
  audio,
  currentOrigin,
  persistedQueue,
  userId,
}: {
  activeSong: MutableRefObject<LibrarySong>;
  audio: MutableRefObject<HTMLAudioElement | null>;
  currentOrigin: MutableRefObject<QueueOrigin>;
  persistedQueue: MutableRefObject<{ id: string; revision: number } | null>;
  userId?: string;
}) {
  const queryClient = useQueryClient();
  const sessionId = useRef(createSessionId());
  const telemetry = useRef<PlaybackTelemetry>({
    started: false,
    qualified: false,
    completed: false,
    listenedSeconds: 0,
    lastPosition: 0,
    lastUpdateMs: 0,
    seeking: false,
  });
  const send = useCallback(
    (eventType: PlaybackEventType, position?: number, duration?: number) => {
      const song = activeSong.current;
      // Listening history and recommendations cover the local library.
      if (!song.id || !userId || isStreamingId(song.id)) return;
      void recordPlaybackEvent({
        event_key: `${sessionId.current}:${song.id}:${eventType}:${Date.now()}`,
        song_id: song.id,
        event_type: eventType,
        session_id: sessionId.current,
        queue_id: persistedQueue.current?.id,
        source: currentOrigin.current,
        position_seconds: position ?? audio.current?.currentTime ?? 0,
        duration_seconds: duration ?? audio.current?.duration ?? song.duration,
      })
        .then(() => {
          if (eventType === "play_started") return;
          void Promise.all([
            queryClient.invalidateQueries({
              queryKey: ["library", "feed"],
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
    [activeSong, audio, currentOrigin, persistedQueue, queryClient, userId],
  );
  /** Records feedback about any song (a Radio pick that has not played). */
  const sendSongEvent = useCallback(
    (songId: string, eventType: PlaybackEventType) => {
      if (!songId || !userId) return Promise.resolve();
      return recordPlaybackEvent({
        event_key: `${sessionId.current}:${songId}:${eventType}:${Date.now()}`,
        song_id: songId,
        event_type: eventType,
        session_id: sessionId.current,
        source: "generated",
      })
        .then(() => {
          void queryClient
            .invalidateQueries({
              queryKey: ["library", "feed"],
              refetchType: "none",
            })
            .catch(() => {});
        })
        .catch(() => {});
    },
    [queryClient, userId],
  );
  return { sendPlaybackEvent: send, sendSongEvent, telemetry };
}
