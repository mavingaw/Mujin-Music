"use client";

import getBaseURL from "@/lib/api/server-url";
import {
  applyPlaybackCapabilities,
  getMediaToken,
  type PlaybackCapabilities,
} from "@parson/music-sdk";

/**
 * Builds a media stream URL. With `capabilities` the server sends original
 * bytes when the player decodes them and otherwise converts losslessly
 * (DSD, WavPack, APE, ALAC on Chromium, ...); without them it always sends
 * the original file, which downloads rely on.
 */
export function buildStreamUrl(
  baseURL: string,
  songId: string,
  bitrate: number = 0,
  slowedReverb: boolean = false,
  mediaToken: string | null = null,
  capabilities: PlaybackCapabilities | null = null,
) {
  const url = new URL(
    `${baseURL}/api/v1/media/songs/${encodeURIComponent(songId)}/stream`,
  );
  url.searchParams.set("bitrate", String(bitrate));
  if (slowedReverb) url.searchParams.set("slowed_reverb", "true");
  applyPlaybackCapabilities(url, capabilities);

  if (mediaToken) {
    url.searchParams.set("media_token", mediaToken);
  }

  return url.toString();
}

export default function streamUrl(
  songId: string,
  bitrate: number = 0,
  slowedReverb: boolean = false,
  capabilities: PlaybackCapabilities | null = null,
) {
  return buildStreamUrl(
    getBaseURL(),
    songId,
    bitrate,
    slowedReverb,
    getMediaToken(),
    capabilities,
  );
}
