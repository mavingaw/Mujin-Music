import {
  chooseTextSource,
  pageTextSources,
  creditRoleLabel,
  creditRolesLabel,
  discographySections,
  enrichmentWaitingLabel,
  focusQueryString,
  focusRoleLabel,
  getAlbumInfos,
  getArtistAbout,
  getArtistDiscography,
  getArtistRelated,
  qualityBadge,
  type FocusSelection,
  getArtistCredits,
  getArtistInfo,
  getArtistWorks,
  lifeSpanFacts,
  type Album,
  type Artist,
  type LibraryAlbum,
} from "@parson/music-sdk";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useLocalSearchParams, useRouter } from "expo-router";
import { ArrowLeft, Play, Shuffle } from "lucide-react-native";
import { useEffect, useMemo, useState } from "react";
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
import { PersonAvatar } from "@/components/person-avatar";
import { Screen, SectionTitle, SongRow } from "@/components/music-ui";
import { AlbumActions } from "@/components/album-actions";
import { EditorialBlock, NoteBlock } from "@/components/credits";
import { CompositionsSection } from "@/components/works";
import { TextSourceChips } from "@/components/text-sources";
import { GenreChips } from "@/components/genres";
import { FocusBar } from "@/components/focus";
import { RelatedPeople } from "@/components/related-people";
import { apiUrl } from "@/lib/runtime";
import { joinMeta } from "@/lib/credit-links";
import { palette } from "@/constants/colors";
import { usePlayer } from "@/providers/player-provider";
import {
  immediateBorderlessPressFeedback,
  immediatePressFeedback,
} from "@/lib/press-feedback";

export default function ArtistScreen() {
  const { id, tab } = useLocalSearchParams<{ id: string; tab?: string }>();
  const router = useRouter();
  const player = usePlayer();
  const [selected, setSelected] = useState<Album | null>(null);
  const artist = useQuery({
    queryKey: ["artist", id],
    queryFn: () => getArtistInfo(id),
    enabled: !!id && !id.startsWith("mb:"),
    retry: 1,
  });
  const about = useQuery({
    queryKey: ["artist-about", id],
    queryFn: () => getArtistAbout(id),
    enabled: !!id,
    staleTime: 60_000,
  });
  const [pickedSource, setPickedSource] = useState<string | null>(null);
  const bioSources = pageTextSources(about.data).filter(
    (source) => source.biography,
  );
  const bioSource = chooseTextSource(bioSources, pickedSource);
  const biography = bioSource?.biography ?? about.data?.biography ?? null;
  const credits = useQuery({
    queryKey: ["artist-credits", id],
    queryFn: () => getArtistCredits(id),
    enabled: !!id,
    staleTime: 60_000,
  });
  const [selection, setSelection] = useState<FocusSelection>({});
  // A person opened under another of their ids moves to their one page.
  const canonical =
    about.data && about.data.artist_id !== id && about.data.requested_id === id
      ? about.data.artist_id
      : null;
  useEffect(() => {
    if (canonical) router.replace(`/artist/${encodeURIComponent(canonical)}`);
  }, [canonical, router]);
  const discography = useQuery({
    queryKey: ["artist-discography", id, focusQueryString(selection)],
    queryFn: () => getArtistDiscography(id, selection),
    enabled: !!id && !canonical,
    placeholderData: keepPreviousData,
    staleTime: 60_000,
  });
  const focusSections = useMemo(
    () =>
      discography.data ? discographySections(discography.data.albums) : null,
    [discography.data],
  );
  const isComposer = Boolean(
    about.data?.roles.some((role) => role.role === "composer"),
  );
  const works = useQuery({
    queryKey: ["artist-works", id],
    queryFn: () => getArtistWorks(id),
    enabled: !!id && isComposer,
    staleTime: 60_000,
  });
  const related = useQuery({
    queryKey: ["artist-related", id],
    queryFn: () => getArtistRelated(id),
    enabled: !!id && !canonical,
    staleTime: 60_000,
  });
  const sections = useMemo(() => {
    const data = artist.data;
    if (!data) return [];
    const sort = (albums: Album[]) =>
      [...albums].sort((a, b) => {
        const left = Date.parse(a.first_release_date || "");
        const right = Date.parse(b.first_release_date || "");
        return (
          (Number.isFinite(right) ? right : -Infinity) -
          (Number.isFinite(left) ? left : -Infinity)
        );
      });
    return data.discography?.length
      ? data.discography.map((section) => ({
          ...section,
          albums: sort(section.albums),
        }))
      : [{ key: "albums", title: "Albums", albums: sort(data.albums) }];
  }, [artist.data]);
  const featuredAlbumIds = useMemo(
    () => artist.data?.featured_on_album_ids ?? [],
    [artist.data?.featured_on_album_ids],
  );
  const featured = useQuery({
    queryKey: ["artist-featured-albums", id, featuredAlbumIds],
    queryFn: () =>
      getAlbumInfos(featuredAlbumIds, false) as Promise<
        Record<string, LibraryAlbum>
      >,
    enabled: featuredAlbumIds.length > 0,
  });
  const songs = useMemo(() => {
    const seen = new Set<string>();
    return sections.flatMap((section) =>
      section.albums.flatMap((album) =>
        album.songs.filter((song) => {
          if (seen.has(song.id)) return false;
          seen.add(song.id);
          return true;
        }),
      ),
    );
  }, [sections]);
  const appearances = useMemo(() => {
    const data = artist.data;
    if (!data) return [];
    return featuredAlbumIds.flatMap((albumId) => {
      const album = featured.data?.[albumId];
      if (!album) return [];
      const track = album.songs.find(
        (song) =>
          song.contributing_artist_ids.includes(data.id) ||
          song.contributing_artists.some(
            (name) =>
              name.localeCompare(data.name, undefined, {
                sensitivity: "base",
              }) === 0,
          ),
      );
      return track ? [{ album, track }] : [];
    });
  }, [artist.data, featured.data, featuredAlbumIds]);
  const contributor: Artist | null =
    !artist.data && credits.data
      ? {
          id: credits.data.artist_id,
          name: credits.data.name,
          icon_url: "",
          followers: 0,
          albums: [],
          featured_on_album_ids: [],
          description: "",
        }
      : null;
  if (
    (artist.isPending && artist.fetchStatus !== "idle") ||
    (!artist.data && credits.isPending)
  )
    return (
      <Screen>
        <ActivityIndicator color="white" style={{ flex: 1 }} />
      </Screen>
    );
  if (!artist.data && !contributor)
    return (
      <Screen>
        <Pressable
          accessibilityRole="button"
          style={styles.errorState}
          onPress={() => void artist.refetch()}
        >
          <Text style={styles.error}>Could not load artist · Tap to retry</Text>
        </Pressable>
      </Screen>
    );
  const data = (artist.data ?? contributor) as Artist;
  const facts = lifeSpanFacts(about.data?.facts);
  const portrait =
    data.icon_url || (about.data?.image ? apiUrl(about.data.image.url) : null);
  const creditedAlbums = credits.data?.albums ?? [];
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
        </View>
        <ScrollView contentContainerStyle={{ paddingBottom: 135 }}>
          <View style={styles.hero}>
            {portrait ? (
              <Artwork
                path={portrait}
                rounded={80}
                size={160}
                style={styles.portrait}
              />
            ) : (
              <PersonAvatar
                name={data.name}
                size={140}
                style={styles.portrait}
              />
            )}
            {contributor ? (
              <Text style={styles.kicker}>Contributor</Text>
            ) : null}
            <Text style={styles.title}>{data.name}</Text>
            {about.data?.facts ? (
              <Text style={styles.facts}>
                {joinMeta([
                  about.data.facts.kind,
                  ...facts.map((fact) => `${fact.label} ${fact.value}`),
                ])}
              </Text>
            ) : null}
            {about.data?.credited_as?.length ? (
              <Text style={styles.facts}>
                Also credited as {about.data.credited_as.join(", ")}
              </Text>
            ) : null}
            <GenreChips centered genres={about.data?.genres} />
            {!data.icon_url && about.data?.image ? (
              <Text style={styles.photoCredit}>
                Photo: {about.data.image.author}, {about.data.image.license},
                via Wikimedia Commons
              </Text>
            ) : null}
            {data.description && !about.data?.biography ? (
              <Text numberOfLines={4} style={styles.description}>
                {data.description}
              </Text>
            ) : null}
            {songs.length ? (
              <View style={styles.actions}>
                <Pressable
                  {...immediateBorderlessPressFeedback}
                  accessibilityLabel={`Shuffle ${data.name}`}
                  accessibilityRole="button"
                  style={styles.shuffle}
                  onPress={() => player.playShuffled(songs)}
                >
                  <Shuffle color="white" size={22} />
                </Pressable>
                <Pressable
                  {...immediatePressFeedback}
                  accessibilityLabel={`Play ${data.name}`}
                  accessibilityRole="button"
                  style={styles.play}
                  onPress={() => songs[0] && player.playSong(songs[0], songs)}
                >
                  <Play color="black" fill="black" size={27} />
                </Pressable>
              </View>
            ) : null}
          </View>
          {biography && tab !== "credits" ? (
            <View style={styles.bio}>
              <SectionTitle>Biography</SectionTitle>
              <TextSourceChips
                active={bioSource?.id}
                onPick={setPickedSource}
                sources={bioSources}
              />
              <EditorialBlock key={biography.url} text={biography} />
            </View>
          ) : null}
          {!biography &&
          tab !== "credits" &&
          enrichmentWaitingLabel(about.data?.enrichment.task, "artist") ? (
            <Text style={styles.waiting}>
              {enrichmentWaitingLabel(about.data?.enrichment.task, "artist")}
            </Text>
          ) : null}
          {works.data?.works.length ? (
            <View>
              <SectionTitle>Compositions</SectionTitle>
              <CompositionsSection
                catalogue={works.data.catalogue}
                works={works.data.works}
              />
            </View>
          ) : null}
          {related.data && tab !== "credits" ? (
            <RelatedPeople
              name={about.data?.name ?? artist.data?.name ?? ""}
              relations={related.data}
            />
          ) : null}
          {creditedAlbums.length ? (
            <View>
              <SectionTitle>Credits</SectionTitle>
              <Text style={styles.roles}>
                {(credits.data?.roles ?? [])
                  .map(
                    (role) =>
                      `${creditRoleLabel(role.role, role.instrument)} (${role.track_count})`,
                  )
                  .join(" · ")}
              </Text>
              <ScrollView
                contentContainerStyle={styles.rail}
                horizontal
                showsHorizontalScrollIndicator={false}
              >
                {creditedAlbums.map((entry) => (
                  <Pressable
                    accessibilityLabel={`${entry.album.name}, ${creditRolesLabel(entry.roles)}`}
                    accessibilityRole="button"
                    key={entry.album.id}
                    onPress={() => router.push(`/album/${entry.album.id}`)}
                    style={styles.card}
                  >
                    <Artwork
                      path={entry.album.cover_url}
                      size={156}
                      rounded={9}
                    />
                    <Text numberOfLines={2} style={styles.albumName}>
                      {entry.album.name}
                    </Text>
                    <Text numberOfLines={1} style={styles.creditRole}>
                      {creditRolesLabel(entry.roles)}
                    </Text>
                  </Pressable>
                ))}
              </ScrollView>
            </View>
          ) : null}
          {focusSections && discography.data ? (
            <View>
              <SectionTitle>Discography</SectionTitle>
              <FocusBar
                loading={discography.isFetching}
                note={
                  discography.data.albums.every((entry) =>
                    entry.sources.every((source) => source === "local"),
                  )
                    ? "Source: every album here is a local file. Streaming albums join the Source filter once a streaming service adds them to the library."
                    : undefined
                }
                onChange={setSelection}
                selection={selection}
                summary={discography.data.focus}
              />
              {discography.data.albums.length === 0 ? (
                <Pressable
                  accessibilityRole="button"
                  onPress={() => setSelection({})}
                  style={styles.emptyFocus}
                >
                  <Text style={styles.creditRole}>
                    No album matches every filter · Tap to clear
                  </Text>
                </Pressable>
              ) : null}
              {focusSections.map((section) => (
                <View key={section.key}>
                  <Text style={styles.subsection}>{section.title}</Text>
                  <View style={styles.grid}>
                    {section.albums.map((entry) => {
                      const badge = qualityBadge(entry);
                      const roles = entry.roles
                        .filter((role) => role !== "main")
                        .map(focusRoleLabel)
                        .slice(0, 2);
                      return (
                        <Pressable
                          accessibilityLabel={`${entry.album.name}${badge ? `, ${badge}` : ""}`}
                          accessibilityRole="button"
                          key={entry.album.id}
                          onLongPress={() =>
                            setSelected(entry.album as unknown as Album)
                          }
                          onPress={() => {
                            if (!selected)
                              router.push(`/album/${entry.album.id}`);
                          }}
                          style={styles.card}
                        >
                          <Artwork
                            path={entry.album.cover_url}
                            rounded={9}
                            size={156}
                          />
                          <Text numberOfLines={2} style={styles.albumName}>
                            {entry.album.name}
                          </Text>
                          <Text numberOfLines={1} style={styles.creditRole}>
                            {joinMeta([
                              entry.artist.id !== data.id
                                ? entry.artist.name
                                : null,
                              entry.year ? String(entry.year) : null,
                            ])}
                          </Text>
                          <View style={styles.badges}>
                            {badge ? (
                              <Text
                                style={[
                                  styles.badge,
                                  entry.quality === "hires" &&
                                    styles.badgeHires,
                                ]}
                              >
                                {entry.quality === "hires"
                                  ? `HI-RES ${badge}`
                                  : badge}
                              </Text>
                            ) : null}
                            {roles.map((role) => (
                              <Text key={role} style={styles.badge}>
                                {role.toUpperCase()}
                              </Text>
                            ))}
                          </View>
                        </Pressable>
                      );
                    })}
                  </View>
                </View>
              ))}
            </View>
          ) : null}
          {(focusSections ? [] : sections).map((section) => (
            <View key={section.key}>
              <SectionTitle>{section.title}</SectionTitle>
              <View style={styles.grid}>
                {section.albums.map((album) => (
                  <Pressable
                    accessibilityLabel={`${album.name}, ${album.primary_type || "Album"}`}
                    accessibilityRole="button"
                    key={album.id}
                    style={styles.card}
                    onPress={() => {
                      if (!selected) router.push(`/album/${album.id}`);
                    }}
                    onLongPress={() => setSelected(album)}
                  >
                    <Artwork path={album.cover_url} size={156} rounded={9} />
                    <Text numberOfLines={2} style={styles.albumName}>
                      {album.name}
                    </Text>
                  </Pressable>
                ))}
              </View>
            </View>
          ))}
          {appearances.length ? (
            <View>
              <SectionTitle>Appears on</SectionTitle>
              {appearances.map(({ album, track }) => (
                <SongRow
                  key={`${album.id}-${track.id}`}
                  queue={appearances.map((appearance) => appearance.track)}
                  showAlbum
                  song={track}
                />
              ))}
            </View>
          ) : null}
          {about.data ? (
            <View style={styles.bio}>
              <SectionTitle>Your note</SectionTitle>
              <NoteBlock
                entityType="artist"
                id={data.id}
                note={about.data.note}
              />
            </View>
          ) : null}
        </ScrollView>
        {selected ? (
          <AlbumActions
            open
            albumId={selected.id}
            artistId={data.id}
            name={selected.name}
            showArtist={false}
            onClose={() => setSelected(null)}
          />
        ) : null}
      </SafeAreaView>
    </Screen>
  );
}

const styles = StyleSheet.create({
  nav: { height: 48, paddingHorizontal: 20, justifyContent: "center" },
  hero: { alignItems: "center", paddingHorizontal: 20, paddingBottom: 28 },
  portrait: { marginBottom: 20 },
  waiting: {
    color: palette.muted,
    fontSize: 14,
    lineHeight: 20,
    paddingHorizontal: 20,
    paddingBottom: 18,
  },
  title: {
    color: "white",
    fontSize: 34,
    fontWeight: "900",
    letterSpacing: -1,
    textAlign: "center",
  },
  description: {
    color: palette.secondary,
    textAlign: "center",
    lineHeight: 20,
    marginTop: 12,
  },
  actions: {
    flexDirection: "row",
    alignItems: "center",
    gap: 14,
    marginTop: 20,
  },
  shuffle: {
    width: 48,
    height: 48,
    borderRadius: 24,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: palette.elevatedStrong,
  },
  play: {
    width: 58,
    height: 58,
    borderRadius: 29,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: "white",
  },
  grid: {
    flexDirection: "row",
    flexWrap: "wrap",
    paddingHorizontal: 20,
    gap: 16,
  },
  card: { width: 156 },
  albumName: {
    color: "white",
    fontWeight: "700",
    marginTop: 8,
    lineHeight: 18,
  },
  kicker: {
    color: palette.muted,
    fontSize: 11,
    fontWeight: "800",
    letterSpacing: 1.2,
    marginBottom: 6,
    textTransform: "uppercase",
  },
  facts: {
    color: palette.secondary,
    fontSize: 13,
    marginTop: 8,
    textAlign: "center",
  },
  photoCredit: {
    color: palette.muted,
    fontSize: 11,
    marginTop: 6,
    textAlign: "center",
  },
  bio: { marginBottom: 12 },
  roles: {
    color: palette.secondary,
    fontSize: 13,
    paddingHorizontal: 20,
    marginBottom: 12,
  },
  rail: { paddingHorizontal: 20, gap: 16 },
  subsection: {
    color: "white",
    fontSize: 16,
    fontWeight: "800",
    paddingHorizontal: 20,
    marginTop: 8,
    marginBottom: 12,
  },
  emptyFocus: { paddingHorizontal: 20, paddingVertical: 16 },
  badges: { flexDirection: "row", flexWrap: "wrap", gap: 4, marginTop: 5 },
  badge: {
    color: palette.secondary,
    fontSize: 9,
    fontWeight: "800",
    letterSpacing: 0.6,
    paddingHorizontal: 5,
    paddingVertical: 1,
    borderRadius: 4,
    overflow: "hidden",
    backgroundColor: "rgba(255,255,255,0.07)",
  },
  badgeHires: { color: "#fcd34d", backgroundColor: "rgba(252,211,77,0.14)" },
  creditRole: { color: palette.muted, fontSize: 12, marginTop: 2 },
  errorState: { flex: 1, alignItems: "center", justifyContent: "center" },
  error: { color: palette.secondary },
});
