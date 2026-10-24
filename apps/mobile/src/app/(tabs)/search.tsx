import {
  getAlbumInfo,
  getStreamingLibrary,
  searchLibrary,
  searchStreaming,
  type CombinedItem,
  type LibraryAlbum,
  type StreamingAlbum,
  type StreamingTrack,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { useRouter } from "expo-router";
import { Play, Search as SearchIcon, X } from "lucide-react-native";
import { useEffect, useMemo, useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { Artwork } from "@/components/artwork";
import { PageTitle, Screen, SongRow } from "@/components/music-ui";
import { AlbumActions } from "@/components/album-actions";
import { ArtistActions } from "@/components/artist-actions";
import { layout, palette } from "@/constants/colors";
import { usePlayer } from "@/providers/player-provider";
import { playableSearchSong } from "@/lib/playable-song";
import { streamingSearchRows } from "@/lib/streaming";
import {
  SourceTag,
  StreamingAlbumRow,
  StreamingTrackRow,
} from "@/components/streaming-ui";

type Scope = "library" | "everywhere";

export default function SearchScreen() {
  const [query, setQuery] = useState("");
  const [deferred, setDeferred] = useState("");
  useEffect(() => {
    const timeout = setTimeout(() => setDeferred(query.trim()), 250);
    return () => clearTimeout(timeout);
  }, [query]);
  const results = useQuery({
    queryKey: ["search", deferred],
    queryFn: () => searchLibrary(deferred),
    enabled: deferred.length > 0,
    staleTime: 60_000,
  });
  const [scope, setScope] = useState<Scope>("everywhere");
  const streaming = useQuery({
    queryKey: ["streaming", "search", deferred],
    queryFn: () => searchStreaming(deferred, { limit: 8 }),
    enabled: deferred.length > 0 && scope === "everywhere",
    staleTime: 60_000,
  });
  const streamingLibrary = useQuery({
    queryKey: ["streaming", "library-search", deferred],
    queryFn: async () => {
      const [albums, tracks] = await Promise.all([
        getStreamingLibrary({ kind: "album", q: deferred, limit: 20 }),
        getStreamingLibrary({ kind: "track", q: deferred, limit: 20 }),
      ]);
      return { albums: albums.items, tracks: tracks.items };
    },
    enabled: deferred.length > 0 && scope === "library",
  });
  const uniqueResults = useMemo(
    () =>
      Array.from(
        new Map(
          (results.data ?? []).map(
            (item) => [`${item.item_type}-${item.id}`, item] as const,
          ),
        ).values(),
      ),
    [results.data],
  );
  return (
    <Screen>
      <SafeAreaView edges={["top"]} style={{ flex: 1 }}>
        <PageTitle>Search</PageTitle>
        <View style={styles.searchBox}>
          <SearchIcon color={palette.secondary} size={21} />
          <TextInput
            accessibilityLabel="Search songs, artists and albums"
            autoFocus={false}
            autoCapitalize="none"
            autoCorrect={false}
            placeholder="Songs, artists and albums"
            placeholderTextColor={palette.secondary}
            style={styles.input}
            value={query}
            onChangeText={setQuery}
          />
          {query ? (
            <Pressable
              accessibilityLabel="Clear search"
              accessibilityRole="button"
              onPress={() => setQuery("")}
            >
              <X color="white" size={20} />
            </Pressable>
          ) : null}
        </View>
        <View accessibilityRole="radiogroup" style={styles.scopes}>
          {(["library", "everywhere"] as const).map((value) => (
            <Pressable
              accessibilityRole="radio"
              accessibilityState={{ selected: scope === value }}
              key={value}
              onPress={() => setScope(value)}
              style={[styles.scope, scope === value && styles.scopeActive]}
            >
              <Text
                style={[
                  styles.scopeText,
                  scope === value && styles.scopeTextActive,
                ]}
              >
                {value === "library" ? "In library" : "Everywhere"}
              </Text>
            </Pressable>
          ))}
        </View>
        {results.isFetching ? (
          <ActivityIndicator color="white" style={{ marginTop: 30 }} />
        ) : results.isError ? (
          <Pressable
            accessibilityRole="button"
            style={styles.retry}
            onPress={() => void results.refetch()}
          >
            <Text style={styles.empty}>Search failed · Tap to try again</Text>
          </Pressable>
        ) : deferred && results.data?.length === 0 ? (
          <Text style={styles.empty}>
            No results for “{deferred}” in your music folder
          </Text>
        ) : null}
        <ScrollView
          keyboardShouldPersistTaps="handled"
          contentContainerStyle={{
            paddingBottom: layout.tabBar + layout.miniPlayer + 24,
          }}
        >
          {uniqueResults.map((item, index) => (
            <SearchResult
              key={`${item.item_type}-${item.id}-${index}`}
              item={item}
            />
          ))}
          {scope === "library"
            ? (() => {
                const albums = (streamingLibrary.data?.albums ?? [])
                  .map((item) => item.item as StreamingAlbum | null)
                  .filter((album): album is StreamingAlbum =>
                    Boolean(album?.id),
                  );
                const tracks = (streamingLibrary.data?.tracks ?? [])
                  .map((item) => item.item as StreamingTrack | null)
                  .filter((track): track is StreamingTrack =>
                    Boolean(track?.id),
                  );
                return (
                  <>
                    {albums.length + tracks.length ? (
                      <Text style={styles.sectionHeading}>
                        Streaming in your library
                      </Text>
                    ) : null}
                    {albums.map((album) => (
                      <StreamingAlbumRow
                        album={{ ...album, in_library: true }}
                        key={album.id}
                      />
                    ))}
                    {tracks.map((track) => (
                      <StreamingTrackRow
                        key={track.id}
                        queue={tracks}
                        track={{ ...track, in_library: true }}
                      />
                    ))}
                  </>
                );
              })()
            : deferred
              ? streamingSearchRows(streaming.data).map((row) =>
                  row.type === "service" ? (
                    <View key={row.key} style={styles.serviceHeader}>
                      <Text style={styles.sectionHeadingInline}>From</Text>
                      <SourceTag large provider={row.provider} />
                      {row.error || row.empty ? (
                        <Text numberOfLines={2} style={styles.serviceNote}>
                          {row.error ?? "No matches"}
                        </Text>
                      ) : null}
                    </View>
                  ) : row.type === "album" ? (
                    <StreamingAlbumRow album={row.album} key={row.key} />
                  ) : row.type === "track" ? (
                    <StreamingTrackRow
                      key={row.key}
                      queue={row.queue}
                      track={row.track}
                    />
                  ) : (
                    <Text key={row.key} style={styles.artistChip}>
                      {row.artist.name}
                    </Text>
                  ),
                )
              : null}
          {scope === "everywhere" && streaming.isFetching ? (
            <ActivityIndicator color="white" style={{ marginTop: 16 }} />
          ) : null}
        </ScrollView>
      </SafeAreaView>
    </Screen>
  );
}

function SearchResult({ item }: { item: CombinedItem }) {
  const router = useRouter();
  const player = usePlayer();
  const type = item.item_type.toLowerCase();
  const [menu, setMenu] = useState(false);
  const [playing, setPlaying] = useState(false);
  const [playFailed, setPlayFailed] = useState(false);
  const path = item.album_object?.cover_url;
  const open = () => {
    if (menu) return;
    if (type.includes("album")) router.push(`/album/${item.id}`);
    else if (type.includes("artist")) router.push(`/artist/${item.id}`);
    else if (type.includes("song")) {
      player.playSong(playableSearchSong(item));
    }
  };
  const playAlbum = async () => {
    if (playing) return;
    setPlayFailed(false);
    setPlaying(true);
    try {
      const album = (await getAlbumInfo(item.id, false)) as LibraryAlbum;
      if (album.songs[0]) player.playSong(album.songs[0], album.songs);
    } catch {
      setPlayFailed(true);
    } finally {
      setPlaying(false);
    }
  };
  if (type.includes("song")) return <SongRow song={playableSearchSong(item)} />;
  return (
    <>
      <View style={styles.result}>
        <Pressable
          accessibilityLabel={`${item.name}, ${item.item_type}`}
          accessibilityRole="button"
          style={({ pressed }) => [
            styles.resultMain,
            pressed && { opacity: 0.6 },
          ]}
          onPress={open}
          onLongPress={() => setMenu(true)}
        >
          {!type.includes("artist") ? (
            <Artwork path={path} size={52} rounded={7} />
          ) : null}
          <View style={{ flex: 1 }}>
            <Text numberOfLines={1} style={styles.resultName}>
              {item.name}
            </Text>
            <Text style={styles.resultType}>
              {item.item_type}
              {item.artist_object?.name ? ` • ${item.artist_object.name}` : ""}
            </Text>
          </View>
        </Pressable>
        {type.includes("album") ? (
          playing ? (
            <ActivityIndicator color="white" />
          ) : (
            <Pressable
              accessibilityLabel={`${playFailed ? "Try playing" : "Play"} ${item.name}`}
              accessibilityRole="button"
              hitSlop={12}
              onPress={(event) => {
                event.stopPropagation();
                void playAlbum();
              }}
            >
              <Play
                color={palette.secondary}
                fill={palette.secondary}
                size={18}
              />
            </Pressable>
          )
        ) : null}
      </View>
      {type.includes("album") ? (
        <AlbumActions
          open={menu}
          onClose={() => setMenu(false)}
          albumId={item.id}
          artistId={item.artist_object?.id}
          name={item.name}
        />
      ) : type.includes("artist") ? (
        <ArtistActions
          open={menu}
          onClose={() => setMenu(false)}
          artistId={item.id}
          name={item.name}
        />
      ) : null}
    </>
  );
}

const styles = StyleSheet.create({
  searchBox: {
    marginHorizontal: 20,
    height: 50,
    borderRadius: 13,
    backgroundColor: palette.elevatedStrong,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 14,
    gap: 10,
    marginBottom: 12,
  },
  input: { flex: 1, color: "white", fontSize: 16, paddingVertical: 0 },
  scopes: {
    flexDirection: "row",
    gap: 8,
    marginHorizontal: 20,
    marginBottom: 10,
  },
  scope: {
    height: 34,
    paddingHorizontal: 14,
    borderRadius: 17,
    borderWidth: 1,
    borderColor: palette.border,
    justifyContent: "center",
  },
  scopeActive: { backgroundColor: "white", borderColor: "white" },
  scopeText: { color: palette.secondary, fontWeight: "700", fontSize: 13 },
  scopeTextActive: { color: "black" },
  sectionHeading: {
    color: "white",
    fontSize: 17,
    fontWeight: "800",
    marginHorizontal: 20,
    marginTop: 22,
    marginBottom: 6,
  },
  sectionHeadingInline: { color: "white", fontSize: 17, fontWeight: "800" },
  serviceHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    marginHorizontal: 20,
    marginTop: 24,
    marginBottom: 6,
  },
  serviceNote: { flex: 1, color: palette.secondary, fontSize: 13 },
  artistChip: {
    color: "white",
    fontWeight: "700",
    marginHorizontal: 20,
    paddingVertical: 10,
  },
  empty: { color: palette.secondary, textAlign: "center", marginTop: 48 },
  retry: { minHeight: 80, alignItems: "center", justifyContent: "center" },
  result: {
    minHeight: 68,
    flexDirection: "row",
    alignItems: "center",
    paddingLeft: 20,
    paddingRight: 20,
  },
  resultMain: {
    minHeight: 68,
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    gap: 13,
  },
  resultName: { color: "white", fontWeight: "700", fontSize: 15 },
  resultType: {
    color: palette.secondary,
    fontSize: 13,
    marginTop: 4,
    textTransform: "capitalize",
  },
});
