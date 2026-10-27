import type {
  InfluencePerson,
  PersonRelations,
  RelatedPerson,
} from "../domain/types";

const plural = (count: number, one: string, many = `${one}s`) =>
  `${count.toLocaleString()} ${count === 1 ? one : many}`;

const capitalize = (value: string) =>
  value ? value.charAt(0).toLocaleUpperCase() + value.slice(1) : value;

const KIND_LABELS: Record<NonNullable<RelatedPerson["kind"]>, string> = {
  conductor: "Conductor",
  ensemble: "Ensemble",
  soloist: "Soloist",
  artist: "Artist",
};

/**
 * What connects a collaborator, as one line:
 * "Piano, composer · 2 albums · 9 tracks".
 */
export function collaboratorLine(person: RelatedPerson) {
  const roles = person.roles
    .slice(0, 2)
    .map((role) => (role === "artist" ? "main artist" : role));
  return [
    roles.length ? capitalize(roles.join(", ")) : null,
    plural(person.album_count, "album"),
    plural(person.track_count, "track"),
  ]
    .filter(Boolean)
    .join(" · ");
}

/**
 * What connects a composer and a performer of their music:
 * "Conductor · 3 recordings · 2 works". `kind` is only set on top
 * performers; the composers someone performs read "2 works · 5 albums".
 */
export function performanceLine(person: RelatedPerson) {
  if (person.kind) {
    return [
      KIND_LABELS[person.kind],
      plural(person.album_count, "recording"),
      plural(person.work_count, "work"),
    ].join(" · ");
  }
  return [
    plural(person.work_count, "work"),
    plural(person.album_count, "album"),
  ].join(" · ");
}

/** One line about someone in Influences or Followers. */
export function influenceLine(person: InfluencePerson) {
  const description = person.description?.trim();
  const short =
    description && description.length > 60
      ? `${description.slice(0, 57).trimEnd()}…`
      : description;
  return [
    person.in_library ? "In your library" : null,
    short ? capitalize(short) : null,
  ]
    .filter(Boolean)
    .join(" · ");
}

/** The related-people sections a page shows, in display order. */
export type RelatedSectionKey =
  "top_performers" | "performing_the_music_of" | "collaborators" | "influences";

export function relatedSections(
  relations: PersonRelations | null | undefined,
): RelatedSectionKey[] {
  if (!relations) return [];
  const sections: RelatedSectionKey[] = [];
  if (relations.top_performers.length) sections.push("top_performers");
  if (relations.performing_the_music_of.length)
    sections.push("performing_the_music_of");
  if (relations.collaborators.length) sections.push("collaborators");
  if (relations.influences.length || relations.followers.length)
    sections.push("influences");
  return sections;
}

/**
 * What the Influences section says when it is empty: Wikidata is only
 * asked once online metadata is on.
 */
export function influencesEmptyLabel(relations: PersonRelations) {
  return relations.enrichment_enabled
    ? "Wikidata lists no influences or followers for this person yet."
    : "Influences and followers come from Wikidata when online metadata is on.";
}
