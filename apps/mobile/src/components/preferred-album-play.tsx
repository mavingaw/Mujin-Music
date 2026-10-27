import {
  SOURCE_PREFERENCE_LABELS,
  albumPlayChoice,
  findAlbumVersions,
  getAlbumInfo,
  getSourcePreference,
  getStreamingAlbum,
  getStreamingProviders,
  rankAlbumVersions,
  streamingTrackToSong,
  type AlbumVersionSubject,
  type LibraryAlbum,
  type LibrarySong,
  type RankedAlbumVersion,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { palette } from "@/constants/colors";
import { usePlayer } from "@/providers/player-provider";
import { useZonePlayback } from "@/hooks/use-zone-playback";

/**
 * Album-level Play that starts the copy the listener's source preference
 * picks (local files, TIDAL or Qobuz), whichever copy's screen is open.
 * Track rows still play the copy they belong to.
 */
export function usePreferredAlbumPlay(
  subject: AlbumVersionSubject | null,
  thisCopy: LibrarySong[],
) {
  const client = useQueryClient();
  const player = usePlayer();
  // Starts go to the zone picked in the zone sheet, like every library Play.
  const playback = useZonePlayback();
  const [starting, setStarting] = useState(false);
  const providers = useQuery({
    queryKey: ["streaming", "providers"],
    queryFn: getStreamingProviders,
    staleTime: 30_000,
  });
  const connected = (providers.data?.providers ?? [])
    .filter((provider) => provider.status === "signed_in")
    .map((provider) => provider.id);
  const preference = useQuery({
    queryKey: ["streaming", "source-preference"],
    queryFn: getSourcePreference,
    staleTime: 5 * 60_000,
  });
  const current = preference.data ?? "highest_quality";
  const versions = useQuery({
    queryKey: ["streaming", "versions", subject?.id ?? "", connected.join(",")],
    queryFn: () => findAlbumVersions(subject!, connected),
    enabled: Boolean(subject) && connected.length > 0,
    staleTime: 5 * 60_000,
  });
  const ranked = versions.data ? rankAlbumVersions(versions.data, current) : [];
  const choice = subject ? albumPlayChoice(subject.id, ranked) : null;
  const ids = [subject?.id, choice?.version.id].filter(Boolean).map(String);
  const albumId = player.current?.album_object?.id;
  const playing =
    player.isPlaying && albumId != null && ids.includes(String(albumId));

  async function songsOf(version: RankedAlbumVersion): Promise<LibrarySong[]> {
    if (version.source === "local") {
      const album = (await client.fetchQuery({
        queryKey: ["album", version.id],
        queryFn: () => getAlbumInfo(version.id, false),
      })) as LibraryAlbum;
      return album.songs ?? [];
    }
    const album = await client.fetchQuery({
      queryKey: ["streaming", "album", version.id],
      queryFn: () => getStreamingAlbum(version.id),
    });
    return album.tracks
      .filter((track) => track.streamable)
      .map((track) => streamingTrackToSong(track, album));
  }

  const playThisCopy = () => {
    if (thisCopy[0]) playback.playSong(thisCopy[0], thisCopy);
  };

  async function play() {
    if (playing) {
      player.toggle();
      return;
    }
    setStarting(true);
    try {
      let next = choice;
      if (!versions.data && subject && connected.length) {
        // Pressed before the copies were known: wait briefly for them.
        const settled = await Promise.race([
          versions.refetch().then((result) => result.data ?? null),
          new Promise<null>((resolve) =>
            setTimeout(() => resolve(null), 4_000),
          ),
        ]);
        next = settled
          ? albumPlayChoice(subject.id, rankAlbumVersions(settled, current))
          : null;
      }
      const best = next?.version;
      if (!best || next?.isThisCopy) {
        playThisCopy();
        return;
      }
      const songs = await songsOf(best);
      if (!songs[0]) throw new Error("empty");
      playback.playSong(songs[0], songs);
    } catch {
      playThisCopy();
    } finally {
      setStarting(false);
    }
  }

  return {
    choice,
    pending: Boolean(subject) && connected.length > 0 && versions.isPending,
    preference: current,
    playing,
    starting,
    play,
    playThisCopy,
  };
}

/** Under the Play button: which copy it starts, and a way to play this one. */
export function PreferredPlayCaption({
  play,
}: {
  play: ReturnType<typeof usePreferredAlbumPlay>;
}) {
  if (play.pending) {
    return <Text style={styles.fine}>Finding the best copy…</Text>;
  }
  const choice = play.choice;
  if (!choice) return null;
  return (
    <View style={styles.caption}>
      <Text style={styles.plays}>Plays {choice.label}</Text>
      <Text style={styles.fine}>
        {SOURCE_PREFERENCE_LABELS[play.preference]}
        {choice.version.reason ? ` · ${choice.version.reason}` : ""}
      </Text>
      {!choice.isThisCopy ? (
        <Pressable
          accessibilityRole="button"
          hitSlop={8}
          onPress={play.playThisCopy}
        >
          <Text style={styles.link}>Play this copy instead</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  caption: { alignItems: "center", gap: 3, marginTop: 10 },
  plays: { color: palette.text, fontSize: 13, fontWeight: "700" },
  fine: { color: palette.muted, fontSize: 12, textAlign: "center" },
  link: {
    color: palette.enhanced,
    fontSize: 13,
    fontWeight: "700",
    marginTop: 2,
  },
});
