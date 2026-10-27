import {
  lyricsFailureKind,
  type LyricsFailure,
  type LyricsResult,
} from "@parson/music-sdk";

/** Why a lyrics request found nothing: no lyrics anywhere, none in the file
 * with online lookups off, the online source down (503), or no answer. */
export function lyricsRequestFailure(error: unknown): LyricsFailure {
  return lyricsFailureKind(error);
}

export function shouldRequestLyrics({
  cachedLyrics,
  completedSongId,
  open,
  songId,
}: {
  cachedLyrics?: LyricsResult;
  completedSongId: string;
  open: boolean;
  songId: string;
}) {
  return Boolean(songId && open && !cachedLyrics && completedSongId !== songId);
}

export function resolveLyricsRenderState({
  cachedLyrics,
  completedLyrics,
  completedSongId,
  localPlainLyrics,
  open,
  songId,
}: {
  cachedLyrics?: LyricsResult;
  completedLyrics: LyricsResult | null;
  completedSongId: string;
  localPlainLyrics?: string;
  open: boolean;
  songId: string;
}) {
  const lyrics =
    completedSongId === songId ? completedLyrics : (cachedLyrics ?? null);
  const hasImmediateLyrics = Boolean(
    lyrics?.plainLyrics || lyrics?.syncedLyrics || localPlainLyrics,
  );

  return {
    lyrics,
    loading:
      Boolean(songId) &&
      open &&
      completedSongId !== songId &&
      !hasImmediateLyrics,
  };
}
