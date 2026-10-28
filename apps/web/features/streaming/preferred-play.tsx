"use client";

import {
  SOURCE_PREFERENCE_LABELS,
  getAlbumInfo,
  getStreamingAlbum,
  type AlbumVersionSubject,
  type LibraryAlbum,
  type RankedAlbumVersion,
} from "@parson/music-sdk";
import { useQueryClient } from "@tanstack/react-query";
import { HardDrive } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { usePlayer } from "@/features/player/player-context";
import { useZonePlayback } from "@/features/player/zones/use-zone-playback";
import { useAlbumVersions } from "./album-versions";
import { ProviderMark } from "./source-badge";
import { useStreamingPlayback } from "./use-streaming";

/**
 * Album-level Play that starts the copy the listener's source preference
 * picks ("TIDAL first", "Highest quality" and so on), whichever copy's page
 * is open. Track rows still play the copy they belong to.
 */
export function usePreferredAlbumPlay(
  subject: AlbumVersionSubject | null,
  playThisCopy: () => void,
) {
  const versions = useAlbumVersions(subject);
  const client = useQueryClient();
  const streaming = useStreamingPlayback();
  const { isPlaying, song: currentSong, togglePlayPause } = usePlayer();
  // Local copies go to the selected zone, like every library Play.
  const playback = useZonePlayback();
  const [starting, setStarting] = useState(false);
  const preferredId = versions.choice?.version.id ?? subject?.id ?? null;
  const currentAlbumId = currentSong?.album_object?.id;
  const playingCopy =
    currentAlbumId != null &&
    [subject?.id, preferredId].some(
      (id) => id != null && String(id) === String(currentAlbumId),
    );

  async function playVersion(version: RankedAlbumVersion) {
    if (version.source === "local") {
      const album = (await client.fetchQuery({
        queryKey: ["albums", version.id],
        queryFn: () => getAlbumInfo(version.id, false),
      })) as LibraryAlbum;
      const artist = album.artist_object;
      const first = album.songs?.[0];
      if (!artist || !first) throw new Error("empty album");
      await playback.playQueue(
        album.songs.map((song) => ({ song, album, artist })),
      );
      return;
    }
    const album = await client.fetchQuery({
      queryKey: ["streaming", "album", version.id],
      queryFn: () => getStreamingAlbum(version.id),
    });
    streaming.playAlbum(album);
  }

  async function play() {
    if (playingCopy && isPlaying) {
      togglePlayPause();
      return;
    }
    if (!subject) {
      playThisCopy();
      return;
    }
    setStarting(true);
    try {
      const ranked = await versions.resolve();
      const best = ranked[0];
      if (!best || ranked.length < 2 || best.id === subject.id) {
        playThisCopy();
        return;
      }
      await playVersion(best);
      toast(`Playing ${copyName(best)}`, {
        description:
          best.reason ?? SOURCE_PREFERENCE_LABELS[versions.preference],
      });
    } catch {
      toast("Your preferred copy could not be opened, so this one is playing.");
      playThisCopy();
    } finally {
      setStarting(false);
    }
  }

  return {
    ...versions,
    play,
    starting,
    /** Whether a copy of this album (this one or the preferred) is playing. */
    playing: playingCopy && isPlaying,
  };
}

function copyName(version: RankedAlbumVersion) {
  const where =
    version.source === "local"
      ? "your own files"
      : version.source === "tidal"
        ? "TIDAL"
        : "Qobuz";
  return version.format?.label
    ? `the ${where} copy (${version.format.label})`
    : `the ${where} copy`;
}

/**
 * Says which copy Play starts, next to the Play button, with a way to play
 * the copy on this page instead.
 */
export function PreferredPlayCaption({
  play,
  onPlayThisCopy,
}: {
  play: ReturnType<typeof usePreferredAlbumPlay>;
  onPlayThisCopy: () => void;
}) {
  const choice = play.choice;
  if (play.pending) {
    return (
      <span className="text-xs text-zinc-500" aria-live="polite">
        Finding the best copy…
      </span>
    );
  }
  if (!choice) return null;
  const { version } = choice;
  return (
    <span className="flex min-w-0 flex-col gap-0.5">
      <span
        className="inline-flex items-center gap-1.5 text-[0.8125rem] font-semibold text-white"
        title={version.reason ?? undefined}
      >
        {version.source === "local" ? (
          <HardDrive aria-hidden="true" className="h-3.5 w-3.5" />
        ) : (
          <ProviderMark provider={version.source} />
        )}
        Plays {choice.label}
      </span>
      <span className="text-xs text-zinc-500">
        {SOURCE_PREFERENCE_LABELS[play.preference]}
        {!choice.isThisCopy && (
          <>
            {" · "}
            <button
              className="text-zinc-300 underline-offset-4 hover:text-white hover:underline"
              onClick={onPlayThisCopy}
              type="button"
            >
              Play this copy
            </button>
          </>
        )}
      </span>
    </span>
  );
}
