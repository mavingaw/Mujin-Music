"use client";

import getBaseURL from "@/lib/api/server-url";
import {
  getMediaToken,
  isStreamingId,
  newPlayId,
  playIdFromStreamUrl,
} from "@parson/music-sdk";

export function buildStreamUrl(
  baseURL: string,
  songId: string,
  bitrate: number = 0,
  slowedReverb: boolean = false,
  mediaToken: string | null = null,
  play: string | null = null,
) {
  const url = new URL(
    `${baseURL}/api/v1/media/songs/${encodeURIComponent(songId)}/stream`,
  );
  url.searchParams.set("bitrate", String(bitrate));
  if (slowedReverb) url.searchParams.set("slowed_reverb", "true");
  // TIDAL and Qobuz streams carry the player's id for this play, so the
  // signal path asks about exactly this stream.
  if (play) url.searchParams.set("play", play);

  if (mediaToken) {
    url.searchParams.set("media_token", mediaToken);
  }

  return url.toString();
}

const MAX_PENDING_PLAYS = 64;
/** Play ids handed out but not yet started, by song and bitrate. */
const pendingPlays = new Map<string, string>();

/**
 * The id for the next play of a streaming track. A preload and the play
 * that promotes it ask twice and get the same id, so the URLs match; once
 * the play starts (`startedStream`), the next play gets a new id.
 */
export function nextPlayId(songId: string, bitrate: number) {
  const key = `${songId}|${bitrate}`;
  const pending = pendingPlays.get(key);
  if (pending) return pending;
  if (pendingPlays.size >= MAX_PENDING_PLAYS) {
    const oldest = pendingPlays.keys().next().value;
    if (oldest !== undefined) pendingPlays.delete(oldest);
  }
  const play = newPlayId();
  pendingPlays.set(key, play);
  return play;
}

/** Marks a stream URL's play as started, so a replay gets a fresh id. */
export function startedStream(url: string) {
  const play = playIdFromStreamUrl(url);
  if (!play) return;
  for (const [key, value] of pendingPlays) {
    if (value === play) pendingPlays.delete(key);
  }
}

export default function streamUrl(
  songId: string,
  bitrate: number = 0,
  slowedReverb: boolean = false,
) {
  return buildStreamUrl(
    getBaseURL(),
    songId,
    bitrate,
    slowedReverb,
    getMediaToken(),
    isStreamingId(songId) ? nextPlayId(songId, bitrate) : null,
  );
}
