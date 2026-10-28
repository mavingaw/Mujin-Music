"use client";

import {
  CREDIT_ROLE_ORDER,
  cleanEditedCredits,
  creditRoleLabel,
  creditTracksLabel,
  editableCredits,
  revertAlbumCredits,
  saveAlbumCredits,
} from "@parson/music-sdk";
import type { AboutCredit, EditedCredit } from "@parson/music-sdk/types";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Check,
  ChevronDown,
  Loader2,
  Lock,
  Plus,
  RotateCcw,
  Trash2,
} from "lucide-react";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

type Row = EditedCredit & { key: number };
type Track = { id: string; name: string; track_number?: number };

let nextKey = 1;
const withKeys = (credits: EditedCredit[]): Row[] =>
  credits.map((credit) => ({ ...credit, key: nextKey++ }));

function errorMessage(error: unknown) {
  return (
    (error as { response?: { data?: { message?: string } } })?.response?.data
      ?.message ?? "Could not save the credits."
  );
}

function TrackPicker({
  row,
  tracks,
  onChange,
}: {
  row: Row;
  tracks: Track[];
  onChange: (trackIds: string[]) => void;
}) {
  const all = row.track_ids.length === 0;
  const toggle = (id: string) => {
    const current = all ? tracks.map((track) => track.id) : row.track_ids;
    const next = current.includes(id)
      ? current.filter((value) => value !== id)
      : [...current, id];
    // Every track is the whole album; no track keeps the last one.
    if (next.length === 0) return;
    onChange(next.length === tracks.length ? [] : next);
  };
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          aria-label={`Tracks for ${row.name || "this credit"}`}
          className="flex h-8 w-full min-w-0 items-center justify-between gap-1 rounded-md border border-white/10 bg-black/30 px-2 text-left text-[0.8125rem] text-zinc-200 hover:border-white/25"
          type="button"
        >
          <span className="truncate">
            {creditTracksLabel(row.track_ids, tracks)}
          </span>
          <ChevronDown aria-hidden className="h-3.5 w-3.5 shrink-0" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="max-h-80 w-72 overflow-y-auto"
      >
        <DropdownMenuCheckboxItem
          checked={all}
          onCheckedChange={() => onChange([])}
          onSelect={(event) => event.preventDefault()}
        >
          All tracks
        </DropdownMenuCheckboxItem>
        <DropdownMenuSeparator />
        {tracks.map((track, index) => (
          <DropdownMenuCheckboxItem
            checked={all || row.track_ids.includes(track.id)}
            key={track.id}
            onCheckedChange={() => toggle(track.id)}
            onSelect={(event) => event.preventDefault()}
          >
            <span className="w-6 shrink-0 tabular-nums text-zinc-500">
              {track.track_number || index + 1}
            </span>
            <span className="truncate">{track.name}</span>
          </DropdownMenuCheckboxItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/**
 * Edits an album's credits as a list: add a person, remove one, change a
 * role or instrument, or narrow a credit to chosen tracks. Saving freezes
 * the list against later lookups until it is reverted.
 */
export function CreditsEditor({
  albumId,
  credits,
  tracks,
  edited,
  onDone,
}: {
  albumId: string;
  credits: readonly AboutCredit[];
  tracks: Track[];
  /** The album already has an edited list. */
  edited: boolean;
  onDone: () => void;
}) {
  const queryClient = useQueryClient();
  const [rows, setRows] = useState<Row[]>(() =>
    withKeys(editableCredits([...credits])),
  );
  const people = useMemo(
    () => [...new Set(credits.map((credit) => credit.name))].sort(),
    [credits],
  );
  const refresh = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: ["album-about", albumId] }),
      queryClient.invalidateQueries({ queryKey: ["albums", albumId] }),
      queryClient.invalidateQueries({ queryKey: ["artist-credits"] }),
      queryClient.invalidateQueries({ queryKey: ["artist-discography"] }),
    ]);
  };
  const save = useMutation({
    mutationFn: () =>
      saveAlbumCredits(
        albumId,
        cleanEditedCredits(rows).map((row) => ({
          name: row.name,
          role: row.role,
          instrument: row.instrument ?? null,
          track_ids: row.track_ids,
          artist_id: row.artist_id ?? null,
        })),
      ),
    onSuccess: async () => {
      await refresh();
      toast.success("Credits saved. Lookups will keep them.");
      onDone();
    },
    onError: (error) => toast(errorMessage(error)),
  });
  const revert = useMutation({
    mutationFn: () => revertAlbumCredits(albumId),
    onSuccess: async () => {
      await refresh();
      toast.success("Credits come from tags and MusicBrainz again.");
      onDone();
    },
    onError: () => toast("Could not revert the credits."),
  });
  const update = (key: number, change: Partial<EditedCredit>) =>
    setRows((current) =>
      current.map((row) => {
        if (row.key !== key) return row;
        const next = { ...row, ...change };
        // A renamed person is someone else; re-link by name on save.
        if (change.name !== undefined && change.name !== row.name)
          next.artist_id = null;
        return next;
      }),
    );
  const busy = save.isPending || revert.isPending;

  return (
    <section aria-label="Edit credits" className="space-y-4">
      <div className="flex flex-wrap items-start justify-between gap-3 rounded-lg border border-mujin-copper/15 bg-mujin-copper/[0.04] px-4 py-3">
        <p className="flex max-w-2xl items-start gap-2 text-[0.8125rem] leading-5 text-zinc-300">
          <Lock
            aria-hidden
            className="mt-0.5 h-3.5 w-3.5 shrink-0 text-mujin-copper"
          />
          Your list replaces the credits from tags and MusicBrainz on this album
          and on every person&apos;s page. It is frozen: looking the album up
          again or identifying it keeps it. Your files are not changed.
        </p>
        {edited && (
          <button
            className="flex shrink-0 items-center gap-1.5 rounded-full border border-white/12 px-3 py-1.5 text-xs font-semibold text-zinc-300 hover:border-white/25 hover:text-white disabled:opacity-50"
            disabled={busy}
            onClick={() => revert.mutate()}
            type="button"
          >
            <RotateCcw className="h-3.5 w-3.5" />
            Revert to tags and MusicBrainz
          </button>
        )}
      </div>
      <datalist id={`credit-people-${albumId}`}>
        {people.map((name) => (
          <option key={name} value={name} />
        ))}
      </datalist>
      <div
        aria-hidden
        className="hidden grid-cols-[minmax(0,1.4fr)_9.5rem_minmax(0,1fr)_minmax(0,1fr)_2rem] gap-2 px-1 text-[0.625rem] font-semibold uppercase tracking-[0.12em] text-zinc-500 sm:grid"
      >
        <span>Person</span>
        <span>Role</span>
        <span>Instrument</span>
        <span>Tracks</span>
        <span />
      </div>
      <ul className="space-y-2 sm:space-y-1.5">
        {rows.map((row) => (
          <li
            className="grid grid-cols-2 gap-2 rounded-lg border border-white/8 bg-white/[0.02] p-2 sm:grid-cols-[minmax(0,1.4fr)_9.5rem_minmax(0,1fr)_minmax(0,1fr)_2rem] sm:items-center sm:border-0 sm:bg-transparent sm:p-1"
            key={row.key}
          >
            <input
              aria-label="Person"
              autoFocus={!row.name}
              className="col-span-2 h-8 min-w-0 rounded-md border border-white/10 bg-black/30 px-2.5 text-[0.8125rem] text-white placeholder:text-zinc-600 focus:border-white/30 focus:outline-none sm:col-span-1"
              list={`credit-people-${albumId}`}
              onChange={(event) =>
                update(row.key, { name: event.target.value })
              }
              placeholder="Name"
              value={row.name}
            />
            <select
              aria-label={`Role for ${row.name || "this credit"}`}
              className="h-8 min-w-0 rounded-md border border-white/10 bg-zinc-950 px-2 text-[0.8125rem] text-white focus:border-white/30 focus:outline-none"
              onChange={(event) =>
                update(row.key, {
                  role: event.target.value,
                  instrument:
                    event.target.value === "performer" ? row.instrument : null,
                })
              }
              value={row.role}
            >
              {CREDIT_ROLE_ORDER.map((role) => (
                <option key={role} value={role}>
                  {creditRoleLabel(role)}
                </option>
              ))}
            </select>
            <input
              aria-label={`Instrument for ${row.name || "this credit"}`}
              className="h-8 min-w-0 rounded-md border border-white/10 bg-black/30 px-2.5 text-[0.8125rem] text-white placeholder:text-zinc-600 focus:border-white/30 focus:outline-none disabled:opacity-40"
              disabled={row.role !== "performer"}
              onChange={(event) =>
                update(row.key, { instrument: event.target.value })
              }
              placeholder={row.role === "performer" ? "e.g. piano" : "—"}
              value={row.instrument ?? ""}
            />
            <TrackPicker
              onChange={(trackIds) => update(row.key, { track_ids: trackIds })}
              row={row}
              tracks={tracks}
            />
            <button
              aria-label={`Remove ${row.name || "this credit"}`}
              className="flex h-8 w-8 items-center justify-center justify-self-end rounded-md text-zinc-500 hover:bg-rose-400/10 hover:text-rose-300"
              onClick={() =>
                setRows((current) =>
                  current.filter((other) => other.key !== row.key),
                )
              }
              type="button"
            >
              <Trash2 className="h-4 w-4" />
            </button>
          </li>
        ))}
      </ul>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <button
          className="flex items-center gap-1.5 rounded-full border border-dashed border-white/20 px-3.5 py-1.5 text-xs font-semibold text-zinc-300 hover:border-white/40 hover:text-white"
          onClick={() =>
            setRows((current) => [
              ...current,
              ...withKeys([{ name: "", role: "performer", track_ids: [] }]),
            ])
          }
          type="button"
        >
          <Plus className="h-3.5 w-3.5" />
          Add person
        </button>
        <div className="flex gap-2">
          <button
            className="rounded-full px-4 py-1.5 text-xs font-semibold text-zinc-300 hover:text-white"
            disabled={busy}
            onClick={onDone}
            type="button"
          >
            Cancel
          </button>
          <button
            className="flex items-center gap-1.5 rounded-full bg-white px-4 py-1.5 text-xs font-semibold text-black hover:bg-zinc-200 disabled:opacity-50"
            disabled={busy}
            onClick={() => save.mutate()}
            type="button"
          >
            {save.isPending ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Check className="h-3.5 w-3.5" />
            )}
            Save credits
          </button>
        </div>
      </div>
    </section>
  );
}
