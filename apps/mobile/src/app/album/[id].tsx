import {
  albumFormat,
  albumHeroYear,
  resolvedFact,
  albumTextPanels,
  chooseTextSource,
  pageTextSources,
  getAlbumAbout,
  getAlbumInfo,
  type AboutCredit,
  type ReleaseMatchCheck,
  type LibraryAlbum,
  unverifiedMatch,
  isStreamingId,
  localAlbumFormat,
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

import { AlbumFacts } from "@/components/album-facts";
import { Artwork } from "@/components/artwork";
import { AlbumActions } from "@/components/album-actions";
import { PauseGlyph } from "@/components/pause-glyph";
import { AlbumFocusTracks } from "@/components/album-focus-tracks";
import { Screen, SectionTitle } from "@/components/music-ui";
import {
  CreditsSection,
  EditorialBlock,
  NoteBlock,
} from "@/components/credits";
import { joinMeta } from "@/lib/credit-links";
import { GenreChips } from "@/components/genres";
import { UnverifiedMatchNotice } from "@/components/release-match";
import { ReviewScoresBlock, TextSourceChips } from "@/components/text-sources";
import {
  PreferredPlayCaption,
  usePreferredAlbumPlay,
} from "@/components/preferred-album-play";
import { StreamingAlbumScreen } from "@/components/streaming-album-screen";
import { fonts, palette } from "@/constants/colors";
import { useZonePlayback } from "@/hooks/use-zone-playback";
import { formatCollectionDuration } from "@/lib/format";
import {
  immediateBorderlessPressFeedback,
  immediatePressFeedback,
} from "@/lib/press-feedback";

export default function AlbumScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  if (isStreamingId(id)) return <StreamingAlbumScreen id={id} />;
  return <LibraryAlbumScreen id={id} />;
}

function LibraryAlbumScreen({ id }: { id: string }) {
  const router = useRouter();
  const playback = useZonePlayback();
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
  const [pickedSource, setPickedSource] = useState<string | null>(null);
  const loaded = album.data;
  const preferred = usePreferredAlbumPlay(
    loaded
      ? {
          id: String(loaded.id),
          title: loaded.name,
          artistName:
            loaded.artist_object?.name ??
            loaded.contributing_artists?.[0] ??
            "",
          source: "local",
          format: localAlbumFormat(loaded.songs ?? []),
        }
      : null,
    loaded?.songs ?? [],
  );
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
  const sources = about.data?.sources;
  // The hero follows the original release, never the edition year.
  const year = sources
    ? (albumHeroYear(about.data, data.first_release_date) ?? undefined)
    : /^\d{4}/.exec(
        (facts?.exact
          ? facts.first_release_date || data.first_release_date
          : data.first_release_date || facts?.first_release_date) || "",
      )?.[0];
  const label = sources
    ? (resolvedFact(about.data, "label")?.value ?? undefined)
    : (data.label ?? (facts?.exact ? facts.labels?.[0]?.name : undefined));
  // Review / About / the pieces' own articles, as on the web.
  const textSources = pageTextSources(about.data).filter(
    (source) => source.background || source.review,
  );
  const textSource = chooseTextSource(textSources, pickedSource);
  const texts = albumTextPanels(about.data, textSource?.id);
  const shownPanel =
    texts.find((entry) => entry.key === textKind) ?? texts[0] ?? null;
  const shownText = shownPanel?.text ?? null;
  const albumPlaying = preferred.playing;
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
                disabled={!data.songs.length || preferred.starting}
                style={styles.circle}
                onPress={() => void preferred.play()}
              >
                {albumPlaying ? (
                  <PauseGlyph color={palette.accentText} size={25} />
                ) : (
                  <Play
                    color={palette.accentText}
                    fill={palette.accentText}
                    size={28}
                  />
                )}
              </Pressable>
              <Pressable
                {...immediateBorderlessPressFeedback}
                accessibilityLabel={`Shuffle ${data.name}`}
                accessibilityRole="button"
                disabled={!data.songs.length}
                style={styles.shuffle}
                onPress={() => playback.playShuffled(data.songs)}
              >
                <Shuffle color={palette.secondary} size={21} />
              </Pressable>
            </View>
            <PreferredPlayCaption play={preferred} />
          </View>
          <AlbumFocusTracks
            album={data}
            albumArtist={artist}
            works={about.data ? (about.data.works ?? null) : undefined}
          />
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
              {!shownPanel?.workMbid ? (
                <TextSourceChips
                  active={textSource?.id}
                  onPick={setPickedSource}
                  sources={textSources}
                />
              ) : null}
              <EditorialBlock key={shownText.url} text={shownText} />
              {about.data?.review_scores ? (
                <ReviewScoresBlock scores={about.data.review_scores} />
              ) : null}
            </View>
          ) : about.data?.review_scores ? (
            <View style={styles.section}>
              <ReviewScoresBlock scores={about.data.review_scores} />
            </View>
          ) : null}
          {sources ? (
            <View style={styles.section}>
              <SectionTitle>Release</SectionTitle>
              <AlbumFacts sources={sources} />
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
    color: palette.text,
    fontFamily: fonts.display,
    fontSize: 34,
    lineHeight: 40,
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
  pillText: { color: "#c7c2b9", fontSize: 12, fontWeight: "700" },
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
    backgroundColor: palette.accent,
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
