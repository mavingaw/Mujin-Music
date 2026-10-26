import {
  albumScope,
  clearedFocus,
  focusCountLabel,
  focusedAlbumTracks,
  getFocusPage,
  isFocused,
  scopedFocus,
  type FocusQuery,
  type LibraryAlbum,
} from "@parson/music-sdk";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { Pressable, StyleSheet, Text } from "react-native";

import { FocusSheet } from "@/components/focus-sheet";
import { LibraryFocusBar } from "@/components/library-focus-bar";
import { SongRow } from "@/components/music-ui";
import { palette } from "@/constants/colors";

/**
 * An album's track list with a Focus scoped to the album, so box sets can
 * be narrowed by performer, conductor, work, format and the rest.
 */
export function AlbumFocusTracks({
  album,
  albumArtist,
}: {
  album: LibraryAlbum;
  albumArtist: string;
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
        tracks.map((song, index) => (
          <SongRow
            albumArtist={albumArtist}
            albumType={album.primary_type}
            index={index}
            key={song.id}
            queue={tracks}
            showAlbum={false}
            song={song}
          />
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
