import { genreAboutWaitingLabel, getGenrePage } from "@parson/music-sdk";
import type { GenreCount, GenrePerson } from "@parson/music-sdk/types";
import { useQuery } from "@tanstack/react-query";
import { useLocalSearchParams, useRouter } from "expo-router";
import { ArrowLeft } from "lucide-react-native";
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
import { EditorialBlock } from "@/components/credits";
import { Screen, SectionTitle } from "@/components/music-ui";
import { PersonAvatar } from "@/components/person-avatar";
import { palette, tint } from "@/constants/colors";
import { artistRoute, genreRoute, joinMeta } from "@/lib/credit-links";
import {
  immediateBorderlessPressFeedback,
  immediatePressFeedback,
} from "@/lib/press-feedback";

function GenreLinks({ genres }: { genres: GenreCount[] }) {
  const router = useRouter();
  if (!genres.length) return null;
  return (
    <View style={styles.chips}>
      {genres.map((genre) => (
        <Pressable
          {...immediatePressFeedback}
          accessibilityRole="link"
          key={genre.slug}
          onPress={() => router.push(genreRoute(genre.slug))}
          style={[styles.chip, genre.album_count === 0 && styles.chipEmpty]}
        >
          <Text style={styles.chipText}>
            {genre.name}
            {genre.album_count ? `  ${genre.album_count}` : ""}
          </Text>
        </Pressable>
      ))}
    </View>
  );
}

function PeopleRail({
  people,
  detail,
}: {
  people: GenrePerson[];
  detail: (person: GenrePerson) => string;
}) {
  const router = useRouter();
  return (
    <ScrollView
      contentContainerStyle={styles.rail}
      horizontal
      showsHorizontalScrollIndicator={false}
    >
      {people.map((person) => (
        <Pressable
          {...immediatePressFeedback}
          accessibilityRole="link"
          key={person.artist_id}
          onPress={() => router.push(artistRoute(person.artist_id))}
          style={styles.person}
        >
          {person.icon_url ? (
            <Artwork path={person.icon_url} rounded={44} size={88} />
          ) : (
            <PersonAvatar
              imageUrl={person.image_url}
              name={person.name}
              size={88}
            />
          )}
          <Text numberOfLines={2} style={styles.personName}>
            {person.name}
          </Text>
          <Text style={styles.personMeta}>{detail(person)}</Text>
        </Pressable>
      ))}
    </ScrollView>
  );
}

/** A genre: About text, sub-genres, artists, composers and albums. */
export default function GenreScreen() {
  const { slug } = useLocalSearchParams<{ slug: string }>();
  const router = useRouter();
  const page = useQuery({
    queryKey: ["genre-page", slug],
    queryFn: () => getGenrePage(slug),
    enabled: !!slug,
    staleTime: 60_000,
  });
  if (page.isPending)
    return (
      <Screen>
        <ActivityIndicator color={palette.text} style={{ flex: 1 }} />
      </Screen>
    );
  if (!page.data)
    return (
      <Screen>
        <Pressable
          accessibilityRole="button"
          style={styles.errorState}
          onPress={() => void page.refetch()}
        >
          <Text style={styles.error}>Could not load genre · Tap to retry</Text>
        </Pressable>
      </Screen>
    );
  const genre = page.data;
  const waiting = genreAboutWaitingLabel(genre.enrichment);
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
            <ArrowLeft color={palette.text} size={25} />
          </Pressable>
        </View>
        <ScrollView contentContainerStyle={{ paddingBottom: 135 }}>
          <View style={styles.hero}>
            <Text style={styles.kicker}>
              {genre.path.length
                ? genre.path.map((ancestor) => ancestor.name).join(" › ")
                : "Genre"}
            </Text>
            <Text accessibilityRole="header" style={styles.title}>
              {genre.name}
            </Text>
            <Text style={styles.meta}>
              {joinMeta([
                `${genre.album_count} ${genre.album_count === 1 ? "album" : "albums"}`,
                genre.artists.length ? `${genre.artists.length} artists` : null,
                genre.composers.length
                  ? `${genre.composers.length} composers`
                  : null,
              ])}
            </Text>
            <GenreLinks
              genres={genre.children.filter((child) => child.album_count > 0)}
            />
          </View>
          {genre.about ? (
            <>
              <SectionTitle>About</SectionTitle>
              <EditorialBlock text={genre.about} />
            </>
          ) : waiting ? (
            <Text style={styles.waiting}>{waiting}</Text>
          ) : null}
          {genre.artists.length ? (
            <>
              <SectionTitle>Artists</SectionTitle>
              <PeopleRail
                detail={(person) =>
                  `${person.album_count} ${person.album_count === 1 ? "album" : "albums"}`
                }
                people={genre.artists}
              />
            </>
          ) : null}
          {genre.composers.length ? (
            <>
              <SectionTitle>Composers</SectionTitle>
              <PeopleRail
                detail={(person) =>
                  `${person.track_count} ${person.track_count === 1 ? "track" : "tracks"}`
                }
                people={genre.composers}
              />
            </>
          ) : null}
          <SectionTitle>{`Your albums (${genre.album_count})`}</SectionTitle>
          {genre.albums.map((album) => (
            <Pressable
              {...immediatePressFeedback}
              accessibilityRole="link"
              key={album.id}
              onPress={() => router.push(`/album/${album.id}`)}
              style={styles.album}
            >
              <Artwork path={album.cover_url} rounded={6} size={56} />
              <View style={{ flex: 1 }}>
                <Text numberOfLines={1} style={styles.albumName}>
                  {album.name}
                </Text>
                <Text numberOfLines={1} style={styles.personMeta}>
                  {joinMeta([album.artist_name, album.year])}
                </Text>
              </View>
            </Pressable>
          ))}
          {genre.related.length ? (
            <>
              <SectionTitle>Related genres</SectionTitle>
              <View style={{ paddingHorizontal: 20 }}>
                <GenreLinks genres={genre.related} />
              </View>
            </>
          ) : null}
        </ScrollView>
      </SafeAreaView>
    </Screen>
  );
}

const styles = StyleSheet.create({
  nav: {
    height: 50,
    paddingHorizontal: 20,
    flexDirection: "row",
    alignItems: "center",
  },
  hero: { paddingHorizontal: 20, paddingBottom: 12 },
  kicker: {
    color: palette.muted,
    fontSize: 11,
    fontWeight: "800",
    letterSpacing: 1.2,
    textTransform: "uppercase",
  },
  title: {
    color: palette.text,
    fontSize: 32,
    lineHeight: 36,
    fontWeight: "900",
    letterSpacing: -0.6,
    marginTop: 8,
  },
  meta: { color: palette.secondary, fontSize: 13, marginTop: 6 },
  chips: { flexDirection: "row", flexWrap: "wrap", gap: 6, marginTop: 12 },
  chip: {
    borderColor: tint(0.14),
    borderRadius: 999,
    borderWidth: StyleSheet.hairlineWidth,
    backgroundColor: tint(0.06),
    paddingHorizontal: 10,
    paddingVertical: 4,
  },
  chipEmpty: { opacity: 0.5 },
  chipText: { color: palette.secondary, fontSize: 12, fontWeight: "600" },
  waiting: { color: palette.muted, fontSize: 13, marginHorizontal: 20 },
  rail: { gap: 14, paddingHorizontal: 20 },
  person: { width: 96, alignItems: "center" },
  personName: {
    color: palette.text,
    fontSize: 13,
    fontWeight: "700",
    marginTop: 6,
    textAlign: "center",
  },
  personMeta: { color: palette.muted, fontSize: 12, marginTop: 2 },
  album: {
    flexDirection: "row",
    gap: 12,
    alignItems: "center",
    paddingHorizontal: 20,
    paddingVertical: 6,
  },
  albumName: { color: palette.text, fontSize: 15, fontWeight: "700" },
  error: { color: palette.secondary, margin: 30 },
  errorState: { flex: 1, alignItems: "center", justifyContent: "center" },
});
