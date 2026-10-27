import {
  albumQualityLabel,
  homeStats,
  type HomeAlbum,
  type HomeArtist,
  type HomeBestYears,
  type HomeDailyPick,
  type HomeEssentials,
  type HomeGenre,
} from "@parson/music-sdk";
import { Image } from "expo-image";
import { LinearGradient } from "expo-linear-gradient";
import { useRouter } from "expo-router";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { Artwork } from "@/components/artwork";
import { PersonAvatar } from "@/components/person-avatar";
import { alpha, eyebrow, fonts, layout, palette } from "@/constants/colors";
import { apiUrl, imageRequestHeaders, imageUrl } from "@/lib/runtime";

/** "Hi, Mavinga" with the library's stat cards (two per row). */
export function HomeGreeting({
  feed,
  greeting,
  name,
}: {
  feed: HomeEssentials;
  greeting: string;
  name: string;
}) {
  const stats = homeStats(feed);
  return (
    <View style={styles.greeting}>
      <Text style={eyebrow}>{greeting}</Text>
      <Text accessibilityRole="header" style={styles.hello}>
        {name ? `Hi, ${name}` : "Your library"}
      </Text>
      {stats.length > 0 && (
        <View style={styles.stats}>
          {stats.map((stat) => (
            <View key={stat.label} style={styles.stat}>
              <Text numberOfLines={1} style={[eyebrow, styles.statLabel]}>
                {stat.label}
              </Text>
              <View style={styles.statValueRow}>
                <Text style={styles.statValue}>{stat.value}</Text>
                {stat.unit ? (
                  <Text style={styles.statUnit}>{stat.unit}</Text>
                ) : null}
                {stat.accent ? <View style={styles.signalDot} /> : null}
              </View>
            </View>
          ))}
        </View>
      )}
    </View>
  );
}

export function HomeSectionHeader({
  eyebrow: label,
  title,
}: {
  eyebrow?: string;
  title: string;
}) {
  return (
    <View style={styles.sectionHeader}>
      {label ? <Text style={eyebrow}>{label}</Text> : null}
      <Text accessibilityRole="header" style={styles.sectionTitle}>
        {title}
      </Text>
    </View>
  );
}

/** A Daily Picks card: blurred cover backdrop, date, reason and covers. */
export function DailyPicksCard({
  date,
  picks,
}: {
  date: string;
  picks: HomeDailyPick[];
}) {
  const router = useRouter();
  const lead = picks[0];
  if (!lead) return null;
  const backdrop = imageUrl(lead.album.cover_url);
  return (
    <View style={styles.picks}>
      {backdrop ? (
        <Image
          blurRadius={40}
          contentFit="cover"
          source={{ uri: backdrop, headers: imageRequestHeaders(backdrop) }}
          style={StyleSheet.absoluteFill}
        />
      ) : null}
      <LinearGradient
        colors={[
          alpha(palette.background, 0.55),
          alpha(palette.background, 0.92),
        ]}
        style={StyleSheet.absoluteFill}
      />
      <Text style={[eyebrow, { color: palette.accent }]}>
        {formatPickDate(date)}
      </Text>
      <Text style={styles.picksTitle}>Daily Picks</Text>
      <Text numberOfLines={2} style={styles.picksReason}>
        {lead.reason}
      </Text>
      <View style={styles.picksRow}>
        {picks.slice(0, 3).map((pick) => (
          <Pressable
            accessibilityLabel={`${pick.album.name} by ${pick.album.artist.name}`}
            accessibilityRole="button"
            key={pick.album.id}
            onPress={() => router.push(`/album/${pick.album.id}`)}
            style={styles.pick}
          >
            <Artwork path={pick.album.cover_url} rounded={6} size={96} />
            <Text numberOfLines={1} style={styles.pickTitle}>
              {pick.album.name}
            </Text>
            <Text numberOfLines={1} style={styles.pickArtist}>
              {pick.album.artist.name}
            </Text>
          </Pressable>
        ))}
      </View>
    </View>
  );
}

function formatPickDate(date: string) {
  const parsed = new Date(`${date}T12:00:00`);
  if (Number.isNaN(parsed.getTime())) return "Today";
  return parsed.toLocaleDateString(undefined, {
    weekday: "long",
    month: "long",
    day: "numeric",
  });
}

/** A horizontal rail of discovery albums with quality badges. */
export function HomeAlbumRail({ albums }: { albums: HomeAlbum[] }) {
  const router = useRouter();
  return (
    <ScrollView
      contentContainerStyle={styles.rail}
      horizontal
      showsHorizontalScrollIndicator={false}
    >
      {albums.map((album) => {
        const label = albumQualityLabel(album.quality);
        return (
          <Pressable
            accessibilityLabel={`${album.name} by ${album.artist.name}`}
            accessibilityRole="button"
            key={album.id}
            onPress={() => router.push(`/album/${album.id}`)}
            style={styles.card}
          >
            <View>
              <Artwork path={album.cover_url} rounded={6} size={150} />
              {label ? (
                <View style={styles.badge}>
                  {album.quality?.hi_res ? (
                    <View style={styles.badgeDot} />
                  ) : null}
                  <Text style={styles.badgeText}>{label}</Text>
                </View>
              ) : null}
            </View>
            <Text numberOfLines={1} style={styles.cardTitle}>
              {album.name}
            </Text>
            <Text numberOfLines={1} style={styles.cardSubtitle}>
              {album.artist.name}
              {album.first_release_date
                ? ` · ${album.first_release_date.slice(0, 4)}`
                : ""}
            </Text>
          </Pressable>
        );
      })}
    </ScrollView>
  );
}

export function ArtistCircles({ artists }: { artists: HomeArtist[] }) {
  const router = useRouter();
  return (
    <ScrollView
      contentContainerStyle={styles.rail}
      horizontal
      showsHorizontalScrollIndicator={false}
    >
      {artists.map((artist) => (
        <Pressable
          accessibilityLabel={artist.name}
          accessibilityRole="button"
          key={artist.id}
          onPress={() => router.push(`/artist/${artist.id}`)}
          style={styles.artist}
        >
          {/* Their photo when online metadata found one; otherwise their
              initials, never a cover posing as a portrait. */}
          <View style={styles.artistPicture}>
            <PersonAvatar
              imageUrl={artist.photo_url}
              name={artist.name}
              size={112}
            />
            {artist.photo_url ? (
              <LinearGradient
                colors={[
                  alpha(palette.accent, 0.0),
                  alpha(palette.accent, 0.28),
                ]}
                pointerEvents="none"
                style={[StyleSheet.absoluteFill, { borderRadius: 56 }]}
              />
            ) : null}
          </View>
          <Text numberOfLines={1} style={styles.artistName}>
            {artist.name}
          </Text>
          <Text numberOfLines={1} style={styles.cardSubtitle}>
            {artist.reason ??
              `${artist.album_count} ${artist.album_count === 1 ? "album" : "albums"}`}
          </Text>
        </Pressable>
      ))}
    </ScrollView>
  );
}

/** "In their prime": the artist's densest run of releases. */
export function BestYearsCard({ prime }: { prime: HomeBestYears }) {
  const router = useRouter();
  const years =
    prime.from_year === prime.to_year
      ? String(prime.from_year)
      : `${prime.from_year}–${prime.to_year}`;
  return (
    <View style={styles.prime}>
      <LinearGradient
        colors={[alpha(palette.accent, 0.22), alpha(palette.background, 0)]}
        end={{ x: 1, y: 0 }}
        start={{ x: 0, y: 1 }}
        style={StyleSheet.absoluteFill}
      />
      <Text style={eyebrow}>In their prime</Text>
      <Pressable
        accessibilityRole="link"
        onPress={() => router.push(`/artist/${prime.artist.id}`)}
      >
        <Text style={styles.primeName}>{prime.artist.name}</Text>
      </Pressable>
      <Text style={styles.primeYears}>{years}</Text>
      <HomeAlbumRail albums={prime.albums} />
    </View>
  );
}

export function GenreTiles({ genres }: { genres: HomeGenre[] }) {
  const router = useRouter();
  return (
    <ScrollView
      contentContainerStyle={styles.rail}
      horizontal
      showsHorizontalScrollIndicator={false}
    >
      {genres.map((genre) => {
        const photos = (genre.photos ?? []).slice(0, 3).map(apiUrl);
        const cover = genre.covers[0] ? imageUrl(genre.covers[0]) : null;
        return (
          <Pressable
            accessibilityLabel={`${genre.name}, ${genre.album_count} albums`}
            accessibilityRole="button"
            key={genre.slug}
            onPress={() =>
              router.push(`/genre/${encodeURIComponent(genre.slug)}`)
            }
            style={styles.genre}
          >
            {photos.length ? (
              // Its artists' photos side by side, washed in copper.
              <View style={[StyleSheet.absoluteFill, styles.genrePhotos]}>
                {photos.map((photo) => (
                  <Image
                    contentFit="cover"
                    contentPosition="top"
                    key={photo}
                    source={{ uri: photo, headers: imageRequestHeaders(photo) }}
                    style={styles.genrePhoto}
                  />
                ))}
              </View>
            ) : cover ? (
              <Image
                contentFit="cover"
                source={{ uri: cover, headers: imageRequestHeaders(cover) }}
                style={[StyleSheet.absoluteFill, { opacity: 0.55 }]}
              />
            ) : null}
            <LinearGradient
              colors={[
                "rgba(184,112,63,0.42)",
                alpha(palette.background, 0.92),
              ]}
              style={StyleSheet.absoluteFill}
            />
            <Text numberOfLines={1} style={styles.genreName}>
              {genre.name}
            </Text>
            <Text style={styles.cardSubtitle}>
              {genre.album_count} {genre.album_count === 1 ? "album" : "albums"}
            </Text>
          </Pressable>
        );
      })}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  greeting: {
    paddingHorizontal: layout.horizontal,
    paddingTop: 18,
    paddingBottom: 6,
  },
  hello: {
    color: palette.text,
    fontFamily: fonts.display,
    fontSize: 44,
    lineHeight: 50,
    marginTop: 8,
  },
  stats: {
    borderColor: palette.border,
    borderRadius: 12,
    borderWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    flexWrap: "wrap",
    marginTop: 22,
    overflow: "hidden",
  },
  stat: {
    borderColor: palette.border,
    borderRightWidth: StyleSheet.hairlineWidth,
    borderBottomWidth: StyleSheet.hairlineWidth,
    // Two per row; an odd last card takes the whole row.
    flexBasis: "50%",
    flexGrow: 1,
    paddingHorizontal: 14,
    paddingVertical: 12,
  },
  statLabel: { fontSize: 9.5 },
  statValueRow: {
    alignItems: "baseline",
    flexDirection: "row",
    gap: 4,
    marginTop: 4,
  },
  statValue: {
    color: palette.text,
    fontFamily: fonts.display,
    fontSize: 30,
    fontVariant: ["tabular-nums"],
  },
  statUnit: { color: palette.tertiary, fontSize: 12 },
  signalDot: {
    alignSelf: "center",
    backgroundColor: palette.accent,
    borderRadius: 4,
    height: 8,
    marginLeft: 4,
    width: 8,
  },
  sectionHeader: {
    marginHorizontal: layout.horizontal,
    marginTop: 30,
    marginBottom: 14,
    gap: 6,
  },
  sectionTitle: {
    color: palette.text,
    fontFamily: fonts.title,
    fontSize: 23,
  },
  picks: {
    borderColor: palette.border,
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    marginHorizontal: layout.horizontal,
    marginTop: 24,
    overflow: "hidden",
    padding: 18,
  },
  picksTitle: {
    color: palette.text,
    fontFamily: fonts.display,
    fontSize: 40,
    lineHeight: 46,
    marginTop: 8,
  },
  picksReason: {
    color: palette.secondary,
    fontSize: 13.5,
    lineHeight: 19,
    marginTop: 6,
  },
  picksRow: { flexDirection: "row", gap: 10, marginTop: 16 },
  pick: { width: 96 },
  pickTitle: {
    color: palette.text,
    fontSize: 12.5,
    fontWeight: "500",
    marginTop: 7,
  },
  pickArtist: { color: palette.tertiary, fontSize: 11.5, marginTop: 1 },
  rail: { gap: 14, paddingHorizontal: layout.horizontal },
  card: { width: 150 },
  cardTitle: {
    color: palette.text,
    fontSize: 14,
    fontWeight: "500",
    marginTop: 9,
  },
  cardSubtitle: { color: palette.secondary, fontSize: 12.5, marginTop: 2 },
  badge: {
    alignItems: "center",
    backgroundColor: "rgba(0,0,0,0.62)",
    borderRadius: 4,
    bottom: 7,
    flexDirection: "row",
    gap: 5,
    left: 7,
    paddingHorizontal: 6,
    paddingVertical: 3,
    position: "absolute",
  },
  badgeDot: {
    backgroundColor: palette.accent,
    borderRadius: 3,
    height: 6,
    width: 6,
  },
  badgeText: {
    color: palette.text,
    fontFamily: "monospace",
    fontSize: 10,
  },
  artist: { alignItems: "center", width: 112 },
  artistPicture: { height: 112, width: 112 },
  genrePhotos: { flexDirection: "row", opacity: 0.6 },
  genrePhoto: { flex: 1, height: "100%" },
  artistName: {
    color: palette.text,
    fontSize: 13.5,
    fontWeight: "500",
    marginTop: 9,
  },
  prime: {
    borderColor: palette.border,
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    marginHorizontal: layout.horizontal,
    marginTop: 30,
    overflow: "hidden",
    paddingBottom: 16,
    paddingTop: 18,
  },
  primeName: {
    color: palette.text,
    fontFamily: fonts.display,
    fontSize: 36,
    lineHeight: 42,
    marginHorizontal: 18,
    marginTop: 8,
  },
  primeYears: {
    color: palette.accent,
    fontFamily: fonts.display,
    fontSize: 36,
    lineHeight: 42,
    marginBottom: 14,
    marginHorizontal: 18,
  },
  genre: {
    backgroundColor: palette.surface,
    borderRadius: 10,
    height: 104,
    justifyContent: "flex-end",
    overflow: "hidden",
    padding: 12,
    width: 150,
  },
  genreName: {
    color: palette.text,
    fontFamily: fonts.title,
    fontSize: 20,
  },
});
