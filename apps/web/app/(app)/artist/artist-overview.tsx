"use client";

import type { ArtistAbout } from "@parson/music-sdk/types";
import { enrichmentWaitingLabel } from "@parson/music-sdk";
import Link from "next/link";
import { EditorialText } from "@/features/library/credits/editorial-text";
import { NoteEditor } from "@/features/library/credits/note-editor";
import { lifeSpanFacts } from "@/features/library/credits/credit-roles";
import { roleFilterKey, roleSummaryLabel } from "./artist-credits";

/** Biography, facts, credit roles and the listener's note. */
export function ArtistOverview({
  about,
  isAdmin,
  onShowCredits,
}: {
  about: ArtistAbout;
  isAdmin: boolean;
  onShowCredits: (filter: string | null) => void;
}) {
  const facts = lifeSpanFacts(about.facts);
  return (
    <div className="grid gap-10 md:grid-cols-[minmax(0,1fr)_15rem]">
      <div className="min-w-0 space-y-10">
        <section aria-label="Biography">
          <h3 className="mb-4 text-xl font-semibold text-white">Biography</h3>
          {about.biography ? (
            <EditorialText clampLines={6} text={about.biography} />
          ) : (
            <p className="text-sm leading-6 text-zinc-500">
              {about.enrichment.enabled ? (
                about.enrichment.task?.status === "pending" ? (
                  enrichmentWaitingLabel(about.enrichment.task, "artist")
                ) : (
                  "No biography was found for this artist."
                )
              ) : isAdmin ? (
                <>
                  Biographies come from Wikipedia when online metadata is on.{" "}
                  <Link
                    className="text-zinc-300 underline-offset-2 hover:text-white hover:underline"
                    href="/settings?tab=library"
                  >
                    Turn it on in Settings
                  </Link>
                  .
                </>
              ) : (
                "Biographies appear here when your server administrator turns on online metadata."
              )}
            </p>
          )}
        </section>

        {about.roles.length > 0 && (
          <section aria-label="Credits summary">
            <h3 className="mb-4 text-xl font-semibold text-white">
              Credited as
            </h3>
            <div className="flex flex-wrap gap-2">
              {about.roles.map((summary) => {
                const { label, detail } = roleSummaryLabel(summary);
                return (
                  <button
                    className="rounded-full bg-white/6 px-3.5 py-1.5 text-sm font-medium text-zinc-200 transition-colors hover:bg-white/10 hover:text-white"
                    key={roleFilterKey(summary)}
                    onClick={() => onShowCredits(roleFilterKey(summary))}
                    type="button"
                  >
                    {label}
                    <span className="ml-2 text-xs text-zinc-500">{detail}</span>
                  </button>
                );
              })}
            </div>
          </section>
        )}

        <section aria-label="Your note">
          <h3 className="mb-4 text-xl font-semibold text-white">Your note</h3>
          <NoteEditor
            entityType="artist"
            id={about.artist_id}
            note={about.note}
          />
        </section>
      </div>

      {(facts.length > 0 ||
        about.links.length > 0 ||
        about.facts?.disambiguation) && (
        <dl className="grid h-fit grid-cols-[auto_minmax(0,1fr)] gap-x-5 gap-y-2 text-sm md:border-l md:border-white/8 md:pl-6">
          {about.facts?.kind && (
            <div className="contents">
              <dt className="font-medium text-zinc-500">Type</dt>
              <dd className="text-zinc-200">{about.facts.kind}</dd>
            </div>
          )}
          {facts.map((fact) => (
            <div className="contents" key={fact.label}>
              <dt className="font-medium text-zinc-500">{fact.label}</dt>
              <dd className="min-w-0 text-zinc-200">{fact.value}</dd>
            </div>
          ))}
          {about.facts?.disambiguation && (
            <div className="contents">
              <dt className="font-medium text-zinc-500">Known as</dt>
              <dd className="min-w-0 text-zinc-200">
                {about.facts.disambiguation}
              </dd>
            </div>
          )}
          {about.links.map((link) => (
            <div className="contents" key={link.url}>
              <dt className="font-medium text-zinc-500">
                {link.provider === "musicbrainz" ? "MusicBrainz" : "Wikidata"}
              </dt>
              <dd>
                <a
                  className="text-zinc-300 underline-offset-2 hover:text-white hover:underline"
                  href={link.url}
                  rel="noreferrer"
                  target="_blank"
                >
                  View
                </a>
              </dd>
            </div>
          ))}
        </dl>
      )}
    </div>
  );
}
