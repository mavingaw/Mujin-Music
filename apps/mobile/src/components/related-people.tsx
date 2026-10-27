import {
  collaboratorLine,
  influenceLine,
  performanceLine,
  relatedSections,
  type InfluencePerson,
  type PersonRelations,
  type RelatedPerson,
} from "@parson/music-sdk";
import { useRouter } from "expo-router";
import {
  Linking,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";

import { PersonAvatar } from "@/components/person-avatar";
import { fonts, palette, tint } from "@/constants/colors";
import { immediatePressFeedback } from "@/lib/press-feedback";

/** Each section's tone, as on the web person page: the brand's copper,
 * burnt copper, warm graphite and bronze. */
const TONES = {
  performers: "#b8703f",
  music: "#8c4f2c",
  collaborators: "#3a3531",
  influences: "#7a6040",
} as const;

function Banner({
  kicker,
  title,
  color,
}: {
  kicker: string;
  title: string;
  color: string;
}) {
  return (
    <View style={[styles.banner, { backgroundColor: color }]}>
      <Text numberOfLines={1} style={styles.kicker}>
        {kicker.toLocaleUpperCase()}
      </Text>
      <Text style={styles.title}>{title}</Text>
    </View>
  );
}

function PeopleRail({
  people,
  line,
}: {
  people: readonly RelatedPerson[];
  line: (person: RelatedPerson) => string;
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
          accessibilityLabel={`${person.name}, ${line(person)}`}
          accessibilityRole={person.artist_id ? "button" : "text"}
          disabled={!person.artist_id}
          key={`${person.artist_id ?? ""}-${person.name}`}
          onPress={() =>
            person.artist_id &&
            router.push(`/artist/${encodeURIComponent(person.artist_id)}`)
          }
          {...immediatePressFeedback}
          style={({ pressed }) => [styles.card, pressed && styles.pressed]}
        >
          <PersonAvatar
            imageUrl={person.image_url}
            name={person.name}
            size={96}
          />
          <Text numberOfLines={1} style={styles.name}>
            {person.name}
          </Text>
          <Text numberOfLines={2} style={styles.line}>
            {line(person)}
          </Text>
        </Pressable>
      ))}
    </ScrollView>
  );
}

function InfluenceList({
  title,
  people,
}: {
  title: string;
  people: readonly InfluencePerson[];
}) {
  const router = useRouter();
  if (!people.length) return null;
  return (
    <View style={styles.influences}>
      <Text style={styles.columnTitle}>{title}</Text>
      {people.slice(0, 10).map((person) => {
        const line = influenceLine(person);
        return (
          <Pressable
            accessibilityRole={person.artist_id ? "button" : "link"}
            key={person.wikidata_url}
            onPress={() =>
              person.artist_id
                ? router.push(`/artist/${encodeURIComponent(person.artist_id)}`)
                : Linking.openURL(person.wikidata_url)
            }
            {...immediatePressFeedback}
            style={({ pressed }) => [
              styles.influence,
              pressed && styles.pressed,
            ]}
          >
            <PersonAvatar
              imageUrl={person.image_url}
              name={person.name}
              size={40}
            />
            <View style={styles.influenceText}>
              <Text numberOfLines={1} style={styles.influenceName}>
                {person.name}
              </Text>
              {line ? (
                <Text numberOfLines={1} style={styles.influenceLine}>
                  {line}
                </Text>
              ) : null}
            </View>
          </Pressable>
        );
      })}
    </View>
  );
}

/** Top performers, composers performed, collaborators, and influences and
 * followers on a person screen. */
export function RelatedPeople({
  name,
  relations,
}: {
  name: string;
  relations: PersonRelations;
}) {
  return (
    <View>
      {relatedSections(relations).map((section) => {
        if (section === "top_performers") {
          return (
            <View key={section} style={styles.section}>
              <Banner
                color={TONES.performers}
                kicker={`Performing the music of ${name}`}
                title="Top performers"
              />
              <PeopleRail
                line={performanceLine}
                people={relations.top_performers}
              />
            </View>
          );
        }
        if (section === "performing_the_music_of") {
          return (
            <View key={section} style={styles.section}>
              <Banner
                color={TONES.music}
                kicker={name}
                title="Performing the music of"
              />
              <PeopleRail
                line={performanceLine}
                people={relations.performing_the_music_of}
              />
            </View>
          );
        }
        if (section === "collaborators") {
          return (
            <View key={section} style={styles.section}>
              <Banner
                color={TONES.collaborators}
                kicker={name}
                title="Collaborators"
              />
              <PeopleRail
                line={collaboratorLine}
                people={relations.collaborators}
              />
            </View>
          );
        }
        return (
          <View key={section} style={styles.section}>
            <Banner
              color={TONES.influences}
              kicker={name}
              title="Influences / Followers"
            />
            <InfluenceList people={relations.influences} title="Influences" />
            <InfluenceList people={relations.followers} title="Followers" />
            {relations.influence_source ? (
              <Text style={styles.source}>
                From {relations.influence_source.source} (“influenced by”),{" "}
                {relations.influence_source.license}.
              </Text>
            ) : null}
          </View>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  section: { marginTop: 28 },
  pressed: { opacity: 0.7 },
  banner: {
    marginHorizontal: 20,
    borderRadius: 14,
    paddingHorizontal: 18,
    paddingTop: 34,
    paddingBottom: 18,
    gap: 6,
  },
  kicker: {
    color: tint(0.85),
    fontSize: 11,
    fontWeight: "700",
    letterSpacing: 1.6,
  },
  title: {
    color: palette.text,
    fontFamily: fonts.display,
    fontSize: 30,
    lineHeight: 32,
  },
  rail: { gap: 6, paddingHorizontal: 14, paddingTop: 12 },
  card: { width: 120, alignItems: "center", padding: 6, borderRadius: 12 },
  name: {
    marginTop: 8,
    color: palette.text,
    fontSize: 13,
    fontWeight: "700",
    textAlign: "center",
  },
  line: {
    marginTop: 2,
    color: tint(0.55),
    fontSize: 11,
    lineHeight: 15,
    textAlign: "center",
  },
  influences: { marginTop: 14, paddingHorizontal: 20 },
  columnTitle: {
    marginBottom: 4,
    color: tint(0.5),
    fontSize: 11,
    fontWeight: "700",
    letterSpacing: 1.2,
    textTransform: "uppercase",
  },
  influence: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingVertical: 7,
  },
  influenceText: { flex: 1, minWidth: 0 },
  influenceName: { color: palette.text, fontSize: 15, fontWeight: "600" },
  influenceLine: { color: tint(0.55), fontSize: 12 },
  source: {
    marginTop: 10,
    paddingHorizontal: 20,
    color: tint(0.4),
    fontSize: 11,
  },
});
