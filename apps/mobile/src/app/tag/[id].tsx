import { getTagPage, tagCountsSummary } from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { useLocalSearchParams, useRouter } from "expo-router";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  Text,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { Artwork } from "@/components/artwork";
import {
  BrowseHeader,
  browseStyles as styles,
} from "@/components/browse-screen";
import { Screen, SongRow } from "@/components/music-ui";
import { tagsKey } from "@/components/tag-picker";
import { palette } from "@/constants/colors";
import { tagHex } from "@/lib/browse-collections";
import { playableCatalogSong } from "@/lib/playable-song";
import { immediatePressFeedback } from "@/lib/press-feedback";

function Heading({ title, count }: { title: string; count: number }) {
  return (
    <View style={styles.sectionHeader}>
      <Text style={styles.sectionTitle}>{title}</Text>
      <Text style={styles.sectionMeta}>{count}</Text>
    </View>
  );
}

/** One tag: its albums, artists and tracks. */
export default function TagScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const tagId = Number(id);
  const page = useQuery({
    queryKey: [...tagsKey, "page", tagId],
    queryFn: () => getTagPage(tagId),
    enabled: Number.isInteger(tagId) && tagId > 0,
  });
  if (page.isPending) {
    return (
      <Screen>
        <ActivityIndicator color="white" style={{ flex: 1 }} />
      </Screen>
    );
  }
  if (!page.data) {
    return (
      <Screen>
        <SafeAreaView edges={["top"]} style={{ flex: 1 }}>
          <BrowseHeader kicker="Tag" title="Not found" />
        </SafeAreaView>
      </Screen>
    );
  }
  const { tag, albums, artists, songs } = page.data;
  const color = tagHex(tag);
  const queue = songs.map((song) =>
    playableCatalogSong({ ...song, duration: song.durationSeconds }),
  );
  return (
    <Screen>
      <SafeAreaView edges={["top"]} style={{ flex: 1 }}>
        <ScrollView contentContainerStyle={{ paddingBottom: 140 }}>
          <BrowseHeader
            accent={color}
            kicker={tag.kind === "smart" ? "Smart tag" : "Tag"}
            subtitle={[tagCountsSummary(tag), tag.description]
              .filter(Boolean)
              .join(" · ")}
            title={`#${tag.name}`}
          />
          {albums.length ? (
            <>
              <Heading count={albums.length} title="Albums" />
              <View
                style={{
                  flexDirection: "row",
                  flexWrap: "wrap",
                  paddingHorizontal: 14,
                }}
              >
                {albums.map((album) => (
                  <Pressable
                    {...immediatePressFeedback}
                    accessibilityRole="link"
                    key={album.id}
                    onPress={() => router.push(`/album/${album.id}`)}
                    style={{ width: "50%", padding: 6 }}
                  >
                    <Artwork path={album.coverPath} size={164} />
                    <Text
                      numberOfLines={1}
                      style={[styles.rowTitle, { marginTop: 8 }]}
                    >
                      {album.name}
                    </Text>
                    <Text numberOfLines={1} style={styles.rowMeta}>
                      {album.artistName}
                    </Text>
                  </Pressable>
                ))}
              </View>
            </>
          ) : null}
          {artists.length ? (
            <>
              <Heading count={artists.length} title="Artists" />
              {artists.map((artist) => (
                <Pressable
                  {...immediatePressFeedback}
                  accessibilityRole="link"
                  key={artist.id}
                  onPress={() => router.push(`/artist/${artist.id}`)}
                  style={styles.row}
                >
                  <Artwork path={artist.artworkPath} rounded={26} size={52} />
                  <View style={styles.rowText}>
                    <Text style={styles.rowTitle}>{artist.name}</Text>
                    <Text style={styles.rowMeta}>
                      {artist.albumCount}{" "}
                      {artist.albumCount === 1 ? "album" : "albums"}
                    </Text>
                  </View>
                </Pressable>
              ))}
            </>
          ) : null}
          {queue.length ? (
            <>
              <Heading count={queue.length} title="Tracks" />
              {queue.map((song, index) => (
                <SongRow
                  index={index}
                  key={song.id}
                  queue={queue}
                  song={song}
                />
              ))}
            </>
          ) : null}
          {!albums.length && !artists.length && !queue.length ? (
            <View style={styles.empty}>
              <Text style={styles.emptyTitle}>
                {tag.kind === "smart"
                  ? "Nothing matches yet"
                  : "Nothing tagged yet"}
              </Text>
              <Text style={[styles.emptyText, { color: palette.secondary }]}>
                {tag.kind === "smart"
                  ? "Music that fits the focus appears here by itself."
                  : "Use Tags… in any album, artist or track menu."}
              </Text>
            </View>
          ) : null}
        </ScrollView>
      </SafeAreaView>
    </Screen>
  );
}
