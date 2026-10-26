"use client";

import {
  ALBUM_FIELD_LABELS,
  FACT_SOURCE_LABELS,
  fieldChoice,
  isTextField,
  type FieldChoice,
  type SourcesDraft,
} from "@parson/music-sdk";
import type { AlbumSources, ResolvedFact } from "@parson/music-sdk/types";
import { Lock, LockOpen } from "lucide-react";
import { FactSourceBadge } from "./credits/fact-source-badge";

const PLACEHOLDERS: Partial<Record<ResolvedFact["field"], string>> = {
  original_date: "YYYY-MM-DD",
  release_date: "YYYY-MM-DD",
  label: "Label name",
  catalog_number: "e.g. CL 1355",
  country: "Two letters, e.g. US",
  barcode: "8 to 14 digits",
};

function Candidate({
  label,
  value,
  active,
}: {
  label: string;
  value: string | null;
  active: boolean;
}) {
  return (
    <div className="min-w-0">
      <dt className="text-[10px] font-semibold uppercase tracking-[0.12em] text-zinc-500">
        {label}
      </dt>
      <dd
        className={`truncate text-[13px] ${
          value
            ? active
              ? "text-white"
              : "text-zinc-400"
            : "italic text-zinc-600"
        }`}
        title={value ?? undefined}
      >
        {value ?? "Nothing"}
      </dd>
    </div>
  );
}

function ChoiceControl({
  fact,
  value,
  onChange,
  disabled,
}: {
  fact: ResolvedFact;
  value: FieldChoice;
  onChange: (choice: FieldChoice) => void;
  disabled?: boolean;
}) {
  const options: { key: FieldChoice; label: string }[] = [
    {
      key: "default",
      label: `Default · ${fact.library_default === "tags" ? "Tags" : "Online"}`,
    },
    { key: "tags", label: "Tags" },
    { key: "online", label: "Online" },
  ];
  return (
    <div
      aria-label={`${ALBUM_FIELD_LABELS[fact.field]} source`}
      className="inline-flex shrink-0 rounded-full border border-white/10 bg-black/30 p-0.5"
      role="radiogroup"
    >
      {options.map((option) => (
        <button
          aria-checked={value === option.key}
          className={`rounded-full px-2.5 py-1 text-[11px] font-semibold transition-colors disabled:opacity-50 ${
            value === option.key
              ? "bg-white text-black"
              : "text-zinc-400 hover:text-white"
          }`}
          disabled={disabled}
          key={option.key}
          onClick={() => onChange(option.key)}
          role="radio"
          type="button"
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

/**
 * Roon-style "Metadata preference": for each fact, whether the album shows
 * its file tags or the matched online release, plus a typed value that is
 * frozen against every later lookup.
 */
export function AlbumSourcesPanel({
  sources,
  draft,
  onDraftChange,
  onEditCredits,
}: {
  sources: AlbumSources;
  draft: SourcesDraft;
  onDraftChange: (draft: SourcesDraft) => void;
  /** Opens the credits editor (the credits fields are edited as a list). */
  onEditCredits?: () => void;
}) {
  const setChoice = (fact: ResolvedFact, choice: FieldChoice) =>
    onDraftChange({
      ...draft,
      sources: { ...draft.sources, [fact.field]: choice },
    });
  const setEdit = (fact: ResolvedFact, value: string) =>
    onDraftChange({ ...draft, edits: { ...draft.edits, [fact.field]: value } });

  return (
    <div className="space-y-4">
      <p className="text-sm leading-6 text-zinc-400">
        Choose, field by field, whether this album shows your file tags or the
        matched MusicBrainz release. When the chosen source has nothing, the
        other fills in. A value you type is <strong>frozen</strong>: looking the
        album up again or identifying it never changes it.
      </p>
      {!sources.online_release && (
        <p className="rounded-md border border-white/8 bg-white/[0.03] px-3 py-2 text-[13px] leading-5 text-zinc-400">
          No verified MusicBrainz release backs this album, so only file tags
          and your own values are available.
        </p>
      )}
      {sources.online_release &&
        sources.online_edition &&
        sources.edition_guessed && (
          <p className="rounded-md border border-white/8 bg-white/[0.03] px-3 py-2 text-[13px] leading-5 text-zinc-400">
            The release was matched automatically, so it may be another
            pressing: its edition date, catalog number, country and barcode only
            show where you choose Online. Identify the exact release to trust
            them.
          </p>
        )}
      {sources.online_release && !sources.online_edition && (
        <p className="rounded-md border border-white/8 bg-white/[0.03] px-3 py-2 text-[13px] leading-5 text-zinc-400">
          The matched release may be another edition, so its date, label,
          catalog number, country and barcode are not offered. Identify the
          exact release to use them.
        </p>
      )}
      <ul aria-label="Metadata preference" className="space-y-2">
        {sources.facts.map((fact) => {
          const choice = draft.sources[fact.field] ?? fieldChoice(fact);
          const preferred =
            choice === "default" ? fact.library_default : choice;
          const text = isTextField(fact.field);
          const typed = draft.edits[fact.field] ?? fact.edited ?? "";
          const frozen = text ? typed.trim().length > 0 : fact.edited !== null;
          return (
            <li
              className={`rounded-lg border px-3 py-3 transition-colors ${
                frozen
                  ? "border-sky-300/20 bg-sky-300/[0.04]"
                  : "border-white/8 bg-white/[0.02]"
              }`}
              key={fact.field}
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="text-sm font-semibold text-white">
                    {ALBUM_FIELD_LABELS[fact.field]}
                  </span>
                  <FactSourceBadge fact={fact} />
                </div>
                <ChoiceControl
                  disabled={frozen}
                  fact={fact}
                  onChange={(next) => setChoice(fact, next)}
                  value={choice}
                />
              </div>
              <dl className="mt-2 grid grid-cols-2 gap-3">
                <Candidate
                  active={!frozen && preferred === "tags"}
                  label={FACT_SOURCE_LABELS.tags}
                  value={fact.tags}
                />
                <Candidate
                  active={!frozen && preferred === "online"}
                  label={FACT_SOURCE_LABELS.online}
                  value={fact.online}
                />
              </dl>
              {text ? (
                <div className="mt-2.5 flex items-center gap-2">
                  {frozen ? (
                    <Lock
                      aria-hidden
                      className="h-3.5 w-3.5 shrink-0 text-sky-300"
                    />
                  ) : (
                    <LockOpen
                      aria-hidden
                      className="h-3.5 w-3.5 shrink-0 text-zinc-600"
                    />
                  )}
                  <input
                    aria-label={`Your ${ALBUM_FIELD_LABELS[fact.field].toLowerCase()}`}
                    className="h-8 min-w-0 flex-1 rounded-md border border-white/10 bg-black/30 px-2.5 text-[13px] text-white placeholder:text-zinc-600 focus:border-white/30 focus:outline-none"
                    onChange={(event) => setEdit(fact, event.target.value)}
                    placeholder={`Type your own (${PLACEHOLDERS[fact.field] ?? "value"}) to freeze it`}
                    value={typed}
                  />
                  {frozen && (
                    <button
                      className="shrink-0 rounded-full px-2.5 py-1 text-[11px] font-semibold text-zinc-400 hover:bg-white/8 hover:text-white"
                      onClick={() => setEdit(fact, "")}
                      type="button"
                    >
                      Unfreeze
                    </button>
                  )}
                </div>
              ) : fact.field === "cover" ? (
                <p className="mt-2 text-xs leading-5 text-zinc-500">
                  {frozen
                    ? "An uploaded cover is frozen; upload another under Details to change it."
                    : "Online fetches the release's front cover from the Cover Art Archive when you save."}
                </p>
              ) : (
                <p className="mt-2 flex flex-wrap items-center gap-x-2 text-xs leading-5 text-zinc-500">
                  {frozen
                    ? "Your edited credits list is frozen and replaces both sources."
                    : "Tags shows only the credits in your files; Online adds MusicBrainz credits to them."}
                  {onEditCredits && (
                    <button
                      className="font-semibold text-zinc-300 underline-offset-2 hover:text-white hover:underline"
                      onClick={onEditCredits}
                      type="button"
                    >
                      Edit credits…
                    </button>
                  )}
                </p>
              )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
