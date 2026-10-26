import {
  albumFormat,
  albumTextPanels,
  albumWorkSections,
  getAlbumAbout,
  getAlbumInfo,
  type AboutCredit,
  type ReleaseMatchCheck,
  type LibraryAlbum,
  unverifiedMatch,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { useLocalSearchParams, useRouter } from "expo-router";
import { ArrowLeft, MoreHorizontal, Play, Shuffle } from "lucide-react-native";
import { useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { Artwork } from "@/components/artwork";
import { AlbumActions } from "@/components/album-actions";
import { PauseGlyph } from "@/components/pause-glyph";
import { Screen, SectionTitle, SongRow } from "@/components/music-ui";
import {
  CreditsSection,
  EditorialBlock,
  NoteBlock,
} from "@/components/credits";
import { joinMeta } from "@/lib/credit-links";
import { WorkHeading } from "@/components/works";
import { GenreChips } from "@/components/genres";
import { UnverifiedMatchNotice } from "@/components/release-match";
import { palette } from "@/constants/colors";
import { usePlayer } from "@/providers/player-provider";
import { formatCollectionDuration } from "@/lib/format";
import {
  immediateBorderlessPressFeedback,
  immediatePressFeedback,
} from "@/lib/press-feedback";

export default function AlbumScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const player = usePlayer();
  const album = useQuery({
    queryKey: ["album", id],
    queryFn: () => getAlbumInfo(id, false) as Promise<LibraryAlbum>,
    enabled: !!id,
  });
  const about = useQuery({
    queryKey: ["album-about", id],
    queryFn: () => getAlbumAbout(id),
    enabled: !!id,
    staleTime: 60_000,
  });
  const [actionsOpen, setActionsOpen] = useState(false);
  const [textKind, setTextKind] = useState<string | null>(null);
  if (album.isPending)
    return (
      <Screen>
        <ActivityIndicator color="white" style={{ flex: 1 }} />
      </Screen>
    );
  if (!album.data)
    return (
      <Screen>
        <Pressable
          accessibilityRole="button"
          style={styles.errorState}
          onPress={() => void album.refetch()}
        >
          <Text style={styles.error}>Could not load album · Tap to retry</Text>
        </Pressable>
      </Screen>
    );
  const data = album.data;
  const artist =
    data.artist_object?.name ??
    data.contributing_artists?.[0] ??
    "Unknown artist";
  const totalDuration = formatCollectionDuration(
    data.songs.reduce((sum, song) => sum + (song.duration || 0), 0),
  );
  const credits: AboutCredit[] =
    about.data?.credits ??
    (data.credits ?? []).map((credit) => ({
      ...credit,
      sources: ["tags"],
    }));
  const facts = about.data?.facts;
  const year = /^\d{4}/.exec(
    (facts?.exact
      ? facts.first_release_date || data.first_release_date
      : data.first_release_date || facts?.first_release_date) || "",
  )?.[0];
  const label =
    data.label ?? (facts?.exact ? facts.labels?.[0]?.name : undefined);
  // Review / About / the pieces' own articles, as on the web.
  const texts = albumTextPanels(about.data);
  const shownPanel =
    texts.find((entry) => entry.key === textKind) ?? texts[0] ?? null;
  const shownText = shownPanel?.text ?? null;
  const albumPlaying =
    player.isPlaying && player.current?.album_object?.id === data.id;
  return (
    <Screen>
      <SafeAreaView edges={["top"]} style={{ flex: 1 }}>
        <View style={styles.nav}>
          <Pressable
            {...immediateBorderlessPressFeedback}
            accessibilityLabel="Back"
            accessibilityRole="button"
            hitSlop={12}
            onPress={router.back}
          >
            <ArrowLeft color="white" size={25} />
          </Pressable>
          <Pressable
            {...immediateBorderlessPressFeedback}
            accessibilityLabel="More album actions"
            accessibilityRole="button"
            hitSlop={12}
            onPress={() => setActionsOpen(true)}
          >
            <MoreHorizontal color="white" size={25} />
          </Pressable>
        </View>
        <ScrollView contentContainerStyle={{ paddingBottom: 135 }}>
          <View style={styles.hero}>
            <Artwork
              path={data.cover_url}
              size={260}
              rounded={12}
              style={styles.cover}
            />
            <Text style={styles.title}>{data.name}</Text>
            <Pressable
              {...immediatePressFeedback}
              accessibilityLabel={`View ${artist}`}
              accessibilityRole="button"
              disabled={!data.artist_object?.id}
              style={styles.artistRow}
              onPress={() =>
                data.artist_object?.id &&
                router.push(`/artist/${data.artist_object.id}`)
              }
            >
              <Text style={styles.artist}>{artist}</Text>
            </Pressable>
            <Text style={styles.meta}>
              {joinMeta([
                year,
                label,
                `${data.songs.length} songs, ${totalDuration}`,
              ])}
            </Text>
            {albumFormat(data.songs) ? (
              <Text style={styles.format}>{albumFormat(data.songs)}</Text>
            ) : null}
            <GenreChips genres={about.data?.genres} />
            {unverifiedMatch(about.data) ? (
              <UnverifiedMatchNotice
                check={unverifiedMatch(about.data) as ReleaseMatchCheck}
              />
            ) : null}
            <View style={styles.actions}>
              <Pressable
                {...immediatePressFeedback}
                accessibilityLabel={albumPlaying ? "Pause album" : "Play album"}
                accessibilityRole="button"
                disabled={!data.songs.length}
                style={styles.circle}
                onPress={() =>
                  albumPlaying
                    ? player.toggle()
                    : data.songs[0] &&
                      player.playSong(data.songs[0], data.songs)
                }
              >
                {albumPlaying ? (
                  <PauseGlyph color="black" size={25} />
                ) : (
                  <Play color="black" fill="black" size={28} />
                )}
              </Pressable>
              <Pressable
                {...immediateBorderlessPressFeedback}
                accessibilityLabel={`Shuffle ${data.name}`}
                accessibilityRole="button"
                disabled={!data.songs.length}
                style={styles.shuffle}
                onPress={() => player.playShuffled(data.songs)}
              >
                <Shuffle color={palette.secondary} size={21} />
              </Pressable>
            </View>
          </View>
          {albumWorkSections(
            data.songs.map((song) => song.id),
            about.data?.works,
          ).map((section) => (
            <View
              key={`${section.work?.mbid ?? "track"}-${section.trackIds[0]}`}
            >
              {section.work ? <WorkHeading work={section.work} /> : null}
              {section.trackIds.map((trackId) => {
                const index = data.songs.findIndex(
                  (song) => song.id === trackId,
                );
                const song = data.songs[index];
                if (!song) return null;
                const trackWork = about.data?.works?.find(
                  (entry) => entry.track_id === song.id,
                );
                return (
                  <SongRow
                    key={song.id}
                    song={song}
                    queue={data.songs}
                    index={index}
                    showAlbum={false}
                    albumArtist={artist}
                    albumType={data.primary_type}
                    title={section.work ? trackWork?.part_title : undefined}
                    work={
                      about.data
                        ? (trackWork?.parent ?? trackWork?.work ?? null)
                        : undefined
                    }
                  />
                );
              })}
            </View>
          ))}
          {data.description ? (
            <Text style={styles.description}>{data.description}</Text>
          ) : null}
          {shownText ? (
            <View style={styles.section}>
              <View style={styles.pills}>
                {texts.map((entry) => (
                  <Pressable
                    accessibilityRole="tab"
                    accessibilityState={{
                      selected: shownPanel?.key === entry.key,
                    }}
                    key={entry.key}
                    onPress={() => setTextKind(entry.key)}
                    style={[
                      styles.pill,
                      shownPanel?.key === entry.key && styles.pillActive,
                    ]}
                  >
                    <Text
                      numberOfLines={1}
                      style={[
                        styles.pillText,
                        shownPanel?.key === entry.key && styles.pillTextActive,
                      ]}
                    >
                      {entry.label}
                    </Text>
                  </Pressable>
                ))}
              </View>
              <EditorialBlock text={shownText} />
            </View>
          ) : null}
          {credits.length ? (
            <View style={styles.section}>
              <SectionTitle>Credits</SectionTitle>
              <CreditsSection credits={credits} />
            </View>
          ) : null}
          {about.data ? (
            <View style={styles.section}>
              <SectionTitle>Your note</SectionTitle>
              <NoteBlock
                entityType="album"
                id={data.id}
                note={about.data.note}
              />
            </View>
          ) : null}
        </ScrollView>
        <AlbumActions
          open={actionsOpen}
          onClose={() => setActionsOpen(false)}
          albumId={data.id}
          artistId={data.artist_object?.id}
          name={data.name}
          loaded={data}
          showAlbum={false}
        />
      </SafeAreaView>
    </Screen>
  );
}

const styles = StyleSheet.create({
  nav: {
    height: 50,
    paddingHorizontal: 20,
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    zIndex: 2,
  },
  hero: { paddingHorizontal: 20, alignItems: "flex-start" },
  cover: {
    alignSelf: "center",
    marginVertical: 12,
    shadowColor: "#000",
    shadowOpacity: 0.7,
    shadowRadius: 18,
    elevation: 12,
  },
  title: {
    color: "white",
    fontSize: 30,
    lineHeight: 34,
    fontWeight: "900",
    letterSpacing: -0.8,
    marginTop: 14,
  },
  artistRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    marginTop: 9,
  },
  artist: { color: "white", fontWeight: "800", fontSize: 15 },
  meta: { color: palette.secondary, fontSize: 13, marginTop: 5 },
  format: { color: palette.muted, fontSize: 12, marginTop: 3 },
  section: { marginTop: 18, gap: 4 },
  pills: {
    flexDirection: "row",
    gap: 8,
    paddingHorizontal: 20,
    marginBottom: 10,
  },
  pill: {
    borderRadius: 999,
    paddingHorizontal: 12,
    paddingVertical: 6,
    backgroundColor: "rgba(255,255,255,0.08)",
  },
  pillActive: { backgroundColor: "white" },
  pillText: { color: "#d4d4d8", fontSize: 12, fontWeight: "700" },
  pillTextActive: { color: "black" },
  actions: {
    width: "100%",
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    marginVertical: 22,
  },
  circle: {
    width: 56,
    height: 56,
    borderRadius: 28,
    backgroundColor: "white",
    alignItems: "center",
    justifyContent: "center",
  },
  shuffle: {
    width: 42,
    height: 42,
    borderRadius: 21,
    alignItems: "center",
    justifyContent: "center",
  },
  description: {
    color: palette.secondary,
    fontSize: 14,
    lineHeight: 21,
    margin: 20,
  },
  error: { color: palette.secondary, margin: 30 },
  errorState: { flex: 1, alignItems: "center", justifyContent: "center" },
});
