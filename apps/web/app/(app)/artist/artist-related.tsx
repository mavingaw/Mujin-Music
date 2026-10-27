"use client";

import {
  collaboratorLine,
  influenceLine,
  influencesEmptyLabel,
  performanceLine,
  personTone,
  personInitials,
  relatedSections,
} from "@parson/music-sdk";
import type {
  InfluencePerson,
  PersonRelations,
  RelatedPerson,
} from "@parson/music-sdk/types";
import { ExternalLink } from "lucide-react";
import Image from "next/image";
import Link from "next/link";
import { useState } from "react";
import {
  PersonAvatar,
  editorialImageSrc,
} from "@/features/library/credits/person-avatar";

/** A color for each section's banner, after Roon's editorial panels. */
const TONES = {
  performers: { base: "#3b3fe0", deep: "#1b1d7a" },
  music: { base: "#e2483d", deep: "#7a1a14" },
  collaborators: { base: "#3f4652", deep: "#16191f" },
  influences: { base: "#0f8a74", deep: "#063d33" },
} as const;

type Tone = (typeof TONES)[keyof typeof TONES];

const personHref = (id: string) => `/artist?id=${encodeURIComponent(id)}`;

/** A tall duotone portrait: the photo in the banner's color, or the
 * person's initials on it when there is no photo. */
function Portrait({
  name,
  imageUrl,
  tone,
}: {
  name: string;
  imageUrl: string | null;
  tone: Tone;
}) {
  const src = editorialImageSrc(imageUrl);
  const [failed, setFailed] = useState(false);
  const [lit] = personTone(name);
  return (
    <>
      {src && !failed ? (
        <Image
          alt=""
          className="object-cover object-top grayscale contrast-125 transition-transform duration-500 group-hover:scale-[1.04]"
          fill
          onError={() => setFailed(true)}
          sizes="(min-width: 768px) 20vw, 33vw"
          src={src}
        />
      ) : (
        <span
          aria-hidden
          className="absolute inset-0 flex items-center justify-center font-serif text-5xl text-white/70"
          style={{
            background: `linear-gradient(160deg, ${lit}, ${tone.deep})`,
          }}
        >
          {personInitials(name)}
        </span>
      )}
      <span
        aria-hidden
        className="absolute inset-0 mix-blend-multiply"
        style={{ background: tone.base }}
      />
      <span
        aria-hidden
        className="absolute inset-0 bg-gradient-to-t from-black/70 via-black/10 to-transparent"
      />
    </>
  );
}

function BannerPerson({
  person,
  tone,
}: {
  person: { artist_id: string | null; name: string; image_url: string | null };
  tone: Tone;
}) {
  const body = (
    <>
      <Portrait imageUrl={person.image_url} name={person.name} tone={tone} />
      <span className="absolute inset-x-2.5 bottom-3 font-serif text-[15px] leading-tight text-white drop-shadow [overflow-wrap:anywhere] [hyphens:auto] sm:inset-x-3 sm:text-2xl">
        {person.name}
      </span>
    </>
  );
  const className =
    "group relative block min-h-44 overflow-hidden sm:min-h-56 md:min-h-64";
  return person.artist_id ? (
    <Link className={className} href={personHref(person.artist_id)}>
      {body}
    </Link>
  ) : (
    <div className={className}>{body}</div>
  );
}

/** The section's banner: a colored panel with the section's name and the
 * first three people as duotone portraits. */
function RelatedBanner({
  kicker,
  title,
  summary,
  people,
  tone,
}: {
  kicker: string;
  title: string;
  summary: string;
  people: readonly {
    artist_id: string | null;
    name: string;
    image_url: string | null;
  }[];
  tone: Tone;
}) {
  // The banner shows three of the first six, those with photos first.
  const shown = [...people.slice(0, 6)]
    .map((person, index) => ({ person, index }))
    .sort(
      (left, right) =>
        Number(!left.person.image_url) - Number(!right.person.image_url) ||
        left.index - right.index,
    )
    .slice(0, 3)
    .map(({ person }) => person);
  const empty = Math.max(0, 3 - shown.length);
  return (
    <div
      className="grid overflow-hidden rounded-2xl shadow-2xl shadow-black/40 md:grid-cols-[minmax(0,1fr)_minmax(0,1.55fr)]"
      style={{ background: tone.base }}
    >
      <div className="flex min-h-44 flex-col justify-end gap-3 p-6 sm:p-8">
        <p className="text-xs font-semibold uppercase tracking-[0.16em] text-white/85">
          {kicker}
        </p>
        <h3 className="font-serif text-4xl leading-[0.95] text-white sm:text-5xl">
          {title}
        </h3>
        <p className="max-w-sm text-sm leading-6 text-white/75">{summary}</p>
      </div>
      <div className="grid grid-cols-3">
        {shown.map((person) => (
          <BannerPerson
            key={`${person.artist_id ?? ""}-${person.name}`}
            person={person}
            tone={tone}
          />
        ))}
        {Array.from({ length: empty }, (_, index) => (
          <span
            aria-hidden
            className="min-h-44 sm:min-h-56 md:min-h-64"
            key={index}
            style={{
              background: `linear-gradient(${150 + index * 30}deg, ${tone.base}, ${tone.deep})`,
              opacity: 0.55 - index * 0.15,
            }}
          />
        ))}
      </div>
    </div>
  );
}

function PersonCard({ person, line }: { person: RelatedPerson; line: string }) {
  const body = (
    <>
      <PersonAvatar
        className="mx-auto shadow-lg shadow-black/40 transition-transform group-hover:scale-[1.03]"
        imageUrl={person.image_url}
        name={person.name}
        size={112}
      />
      <span className="mt-3 block truncate text-center text-sm font-semibold text-white">
        {person.name}
      </span>
      <span className="mt-0.5 line-clamp-2 block text-center text-xs leading-5 text-zinc-400">
        {line}
      </span>
    </>
  );
  const className =
    "group w-36 shrink-0 snap-start rounded-xl p-2 transition-colors hover:bg-white/5";
  return person.artist_id ? (
    <Link
      className={className}
      href={personHref(person.artist_id)}
      title={person.albums.map((album) => album.name).join(" · ")}
    >
      {body}
    </Link>
  ) : (
    <div className={className}>{body}</div>
  );
}

function PeopleRow({
  label,
  people,
  line,
}: {
  label: string;
  people: readonly RelatedPerson[];
  line: (person: RelatedPerson) => string;
}) {
  return (
    <ul
      aria-label={label}
      className="-mx-2 mt-4 flex snap-x gap-2 overflow-x-auto pb-2 [scrollbar-width:thin]"
    >
      {people.map((person) => (
        <li key={`${person.artist_id ?? ""}-${person.name}`}>
          <PersonCard line={line(person)} person={person} />
        </li>
      ))}
    </ul>
  );
}

function InfluenceRow({ person }: { person: InfluencePerson }) {
  const line = influenceLine(person);
  const body = (
    <>
      <PersonAvatar imageUrl={person.image_url} name={person.name} size={44} />
      <span className="min-w-0">
        <span className="block truncate text-[15px] font-medium text-zinc-100 group-hover:text-white">
          {person.name}
        </span>
        {line && (
          <span className="block truncate text-xs text-zinc-500">{line}</span>
        )}
      </span>
      {!person.artist_id && (
        <ExternalLink
          aria-hidden
          className="ml-auto h-3.5 w-3.5 shrink-0 text-zinc-600"
        />
      )}
    </>
  );
  const className =
    "group flex min-w-0 items-center gap-3 rounded-lg px-2 py-2 transition-colors hover:bg-white/5";
  return person.artist_id ? (
    <Link className={className} href={personHref(person.artist_id)}>
      {body}
    </Link>
  ) : (
    <a
      className={className}
      href={person.wikidata_url}
      rel="noreferrer"
      target="_blank"
      title="Open on Wikidata"
    >
      {body}
    </a>
  );
}

function InfluenceColumn({
  title,
  people,
}: {
  title: string;
  people: readonly InfluencePerson[];
}) {
  if (!people.length) return null;
  return (
    <div className="min-w-0">
      <h4 className="mb-2 text-xs font-semibold uppercase tracking-[0.12em] text-zinc-500">
        {title}
        <span className="ml-2 text-zinc-600">{people.length}</span>
      </h4>
      <ul className="grid gap-0.5">
        {people.slice(0, 12).map((person) => (
          <li key={person.wikidata_url}>
            <InfluenceRow person={person} />
          </li>
        ))}
      </ul>
    </div>
  );
}

/**
 * The related-people graph under a person's overview: top performers of
 * their music, the composers they perform, collaborators, and influences
 * and followers, each opening that person's page.
 */
export function ArtistRelated({
  name,
  relations,
}: {
  name: string;
  relations: PersonRelations;
}) {
  const sections = relatedSections(relations);
  const upper = name.toLocaleUpperCase();
  return (
    <div className="space-y-12">
      {sections.map((section) => {
        if (section === "top_performers") {
          const people = relations.top_performers;
          return (
            <section aria-label="Top performers" key={section}>
              <RelatedBanner
                kicker={`Performing the music of ${name}`}
                people={people}
                summary={`The conductors, ensembles and soloists on the most recordings of ${name}'s music in your library.`}
                title="Top performers"
                tone={TONES.performers}
              />
              <PeopleRow
                label="Top performers"
                line={performanceLine}
                people={people}
              />
            </section>
          );
        }
        if (section === "performing_the_music_of") {
          const people = relations.performing_the_music_of;
          return (
            <section aria-label="Performing the music of" key={section}>
              <RelatedBanner
                kicker={upper}
                people={people}
                summary={`The composers whose works ${name} records, by how much of their music is in your library.`}
                title="Performing the music of"
                tone={TONES.music}
              />
              <PeopleRow
                label="Performing the music of"
                line={performanceLine}
                people={people}
              />
            </section>
          );
        }
        if (section === "collaborators") {
          const people = relations.collaborators;
          return (
            <section aria-label="Collaborators" key={section}>
              <RelatedBanner
                kicker={upper}
                people={people}
                summary={`The people ${name} made music with, by the releases and tracks they share in your library's credits.`}
                title="Collaborators"
                tone={TONES.collaborators}
              />
              <PeopleRow
                label="Collaborators"
                line={collaboratorLine}
                people={people}
              />
            </section>
          );
        }
        const featured = [
          ...relations.influences.slice(0, 2),
          ...relations.followers,
          ...relations.influences.slice(2),
        ];
        return (
          <section aria-label="Influences and followers" key={section}>
            <RelatedBanner
              kicker={upper}
              people={featured}
              summary={`Who influenced ${name}, and whom ${name} influenced.`}
              title="Influences / Followers"
              tone={TONES.influences}
            />
            <div className="mt-5 grid gap-8 md:grid-cols-2">
              <InfluenceColumn
                people={relations.influences}
                title="Influences"
              />
              <InfluenceColumn people={relations.followers} title="Followers" />
            </div>
            {relations.influence_source && (
              <p className="mt-4 text-xs text-zinc-500">
                From{" "}
                <a
                  className="text-zinc-400 underline-offset-2 hover:text-white hover:underline"
                  href={relations.influence_source.url}
                  rel="noreferrer"
                  target="_blank"
                >
                  {relations.influence_source.source} (“influenced by”)
                </a>
                , available under{" "}
                <a
                  className="text-zinc-400 underline-offset-2 hover:text-white hover:underline"
                  href={relations.influence_source.license_url}
                  rel="noreferrer"
                  target="_blank"
                >
                  {relations.influence_source.license}
                </a>
                .
              </p>
            )}
          </section>
        );
      })}
      {!sections.includes("influences") && sections.length > 0 && (
        <p className="text-xs text-zinc-500">
          {influencesEmptyLabel(relations)}
        </p>
      )}
    </div>
  );
}
