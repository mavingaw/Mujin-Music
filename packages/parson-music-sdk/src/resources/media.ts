import api from "../core/http";
import {
  type PlaybackCapabilities,
  type SongDelivery,
  playbackCapabilityParams,
} from "../player/formats";

export interface SongDeliveryOptions {
  /** Lossy streaming bitrate in kbps; 0 means best quality. */
  bitrate?: number;
  /** A server-side effect, e.g. `slowed_reverb`. */
  audioEffect?: string | null;
}

/**
 * Asks the server how it will deliver a track to a player with these
 * capabilities: original bytes, a lossless conversion, DoP, or a lossy
 * transcode, with every processing step. It makes the same decision as the
 * stream request with the same parameters.
 */
export async function getSongDelivery(
  songId: string,
  capabilities: PlaybackCapabilities | null,
  options: SongDeliveryOptions = {},
): Promise<SongDelivery> {
  const params: Record<string, string> = {
    ...playbackCapabilityParams(capabilities),
    bitrate: String(options.bitrate ?? 0),
  };
  if (options.audioEffect && options.audioEffect !== "none") {
    params.audio_effect = options.audioEffect;
  }
  const response = await api.get<SongDelivery>(
    `/media/songs/${encodeURIComponent(songId)}/delivery`,
    { params, timeout: 8_000 },
  );
  return response.data;
}
