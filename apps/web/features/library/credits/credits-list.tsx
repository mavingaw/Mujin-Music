"use client";

import type { AboutCredit } from "@parson/music-sdk/types";
import Link from "next/link";
import { creditHref, creditTracksLabel, groupCredits } from "./credit-roles";
import { PersonAvatar } from "./person-avatar";

function SourceMark({ sources }: { sources: AboutCredit["sources"] }) {
  if (sources.includes("edited")) {
    return (
      <span
        className="rounded border border-sky-300/25 px-1 py-px text-[10px] font-semibold uppercase tracking-wide text-sky-200"
        title="From the album's edited credits list (frozen against lookups)"
      >
        Edited
      </span>
    );
  }
  if (!sources.includes("musicbrainz")) return null;
  return (
    <span
      className="rounded border border-white/10 px-1 py-px text-[10px] font-semibold uppercase tracking-wide text-zinc-500"
      title={
        sources.includes("tags")
          ? "In your file tags and on MusicBrainz"
          : "From MusicBrainz"
      }
    >
      MB
    </span>
  );
}

export function CreditName({
  credit,
  className = "",
}: {
  credit: { id: string | null; name: string; role: string };
  className?: string;
}) {
  if (!credit.id) return <span className={className}>{credit.name}</span>;
  return (
    <Link
      className={`hover:text-white hover:underline ${className}`}
      href={creditHref(credit.id, credit.role)}
    >
      {credit.name}
    </Link>
  );
}

/**
 * Liner-notes style credits: role headings with the people under them, each
 * linking to everything else they are credited on.
 */
export function CreditsList({
  credits,
  tracks,
  showTracks = true,
  emptyMessage = "No credits yet.",
}: {
  credits: readonly AboutCredit[];
  tracks?: readonly { id: string; track_number?: number }[];
  showTracks?: boolean;
  emptyMessage?: React.ReactNode;
}) {
  const groups = groupCredits(credits);
  if (!groups.length) {
    return <p className="text-sm leading-6 text-zinc-500">{emptyMessage}</p>;
  }
  return (
    <dl className="grid gap-x-10 gap-y-6 sm:grid-cols-2">
      {groups.map((group) => (
        <div className="min-w-0" key={group.key}>
          <dt className="mb-1 text-xs font-semibold uppercase tracking-[0.08em] text-zinc-500">
            {group.label}
          </dt>
          {group.credits.map((credit) => {
            const trackLabel =
              showTracks && tracks
                ? creditTracksLabel(credit.track_ids, tracks)
                : "";
            return (
              <dd
                className="flex min-w-0 items-center gap-3 py-1.5 text-[15px]"
                key={`${credit.id ?? ""}-${credit.name}`}
              >
                <PersonAvatar
                  imageUrl={credit.image_url}
                  name={credit.name}
                  size={36}
                />
                <span className="flex min-w-0 flex-col">
                  <CreditName
                    className="truncate font-medium text-zinc-100"
                    credit={credit}
                  />
                  {credit.credited_as?.length ? (
                    <span
                      className="truncate text-xs text-zinc-500"
                      title="The same person, credited under another name"
                    >
                      as {credit.credited_as.join(", ")}
                    </span>
                  ) : null}
                </span>
                <SourceMark sources={credit.sources} />
                {trackLabel && (
                  <span className="shrink-0 text-xs tabular-nums text-zinc-500">
                    {trackLabel}
                  </span>
                )}
              </dd>
            );
          })}
        </div>
      ))}
    </dl>
  );
}
