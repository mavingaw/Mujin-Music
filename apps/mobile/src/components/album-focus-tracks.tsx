import {
  albumScope,
  albumWorkSections,
  clearedFocus,
  focusCountLabel,
  focusedAlbumTracks,
  getFocusPage,
  isFocused,
  scopedFocus,
  type FocusQuery,
  type LibraryAlbum,
  type TrackWork,
} from "@parson/music-sdk";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { FocusSheet } from "@/components/focus-sheet";
import { LibraryFocusBar } from "@/components/library-focus-bar";
import { SongRow } from "@/components/music-ui";
import { WorkHeading } from "@/components/works";
import { palette } from "@/constants/colors";

/**
 * An album's track list with a Focus scoped to the album, so box sets can
 * be narrowed by performer, conductor, work, format and the rest. Tracks
 * that perform one work are grouped under its heading (from the album's
 * online metadata, once `works` has loaded).
 */
export function AlbumFocusTracks({
  album,
  albumArtist,
  works,
}: {
  album: LibraryAlbum;
  albumArtist: string;
  /** The album's works; `undefined` while they load. */
  works?: readonly TrackWork[] | null;
}) {
  const [focus, setFocus] = useState<FocusQuery>(() =>
    scopedFocus("songs", [albumScope(album.id)]),
  );
  const [open, setOpen] = useState(false);
  const query =
    focus.scope?.[0]?.facet === "album" && focus.scope[0].album_id === album.id
      ? focus
      : scopedFocus("songs", [albumScope(album.id)]);
  const sorted = Boolean(query.sort);
  const active = isFocused(query) || sorted;
  const focused = useQuery({
    queryKey: ["library", "focus", "album-tracks", "mobile", query],
    queryFn: () => getFocusPage(query, 0, 200),
    enabled: active,
    placeholderData: keepPreviousData,
  });
  const tracks = focusedAlbumTracks(
    album.songs,
    active ? focused.data?.songs.map((song) => song.id) : undefined,
    sorted,
  );
  return (
    <>
      {album.songs.length > 1 ? (
        <LibraryFocusBar
          countLabel={
            active && focused.data
              ? focusCountLabel(
                  query,
                  focused.data.total,
                  focused.data.libraryTotal,
                )
              : null
          }
          onOpen={() => setOpen(true)}
          query={query}
          setQuery={setFocus}
        />
      ) : null}
      {active && focused.data && tracks.length === 0 ? (
        <Pressable
          accessibilityRole="button"
          onPress={() => setFocus(clearedFocus(query))}
          style={styles.empty}
        >
          <Text style={styles.emptyText}>
            No track matches this focus · Tap to clear
          </Text>
        </Pressable>
      ) : (
        albumWorkSections(
          tracks.map((song) => song.id),
          works ?? undefined,
        ).map((section) => (
          <View key={`${section.work?.mbid ?? "track"}-${section.trackIds[0]}`}>
            {section.work ? <WorkHeading work={section.work} /> : null}
            {section.trackIds.map((trackId) => {
              const index = tracks.findIndex((song) => song.id === trackId);
              const song = tracks[index];
              if (!song) return null;
              const trackWork = works?.find(
                (entry) => entry.track_id === song.id,
              );
              return (
                <SongRow
                  albumArtist={albumArtist}
                  albumType={album.primary_type}
                  index={index}
                  key={song.id}
                  queue={tracks}
                  showAlbum={false}
                  song={song}
                  title={section.work ? trackWork?.part_title : undefined}
                  work={
                    works === undefined
                      ? undefined
                      : (trackWork?.parent ?? trackWork?.work ?? null)
                  }
                />
              );
            })}
          </View>
        ))
      )}
      <FocusSheet
        bookmarks={false}
        onClose={() => setOpen(false)}
        open={open}
        query={query}
        setQuery={setFocus}
      />
    </>
  );
}

const styles = StyleSheet.create({
  empty: { padding: 24, alignItems: "center" },
  emptyText: { color: palette.secondary },
});
