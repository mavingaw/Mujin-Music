"use client";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { cn } from "@/lib/cn";
import {
  bulkTag,
  createTag,
  getTagCoverage,
  getTags,
  tagChanges,
  tagCheckState,
  type TagCheckState,
  type TaggedEntity,
  type UserTag,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Loader2, Minus, Plus, Search, Sparkles } from "lucide-react";
import { useMemo, useState, type FormEvent } from "react";
import { toast } from "sonner";
import { TAGS_KEY } from "@/features/library/focus/tag-submenu";
import { tagTone } from "./tag-colors";

const NOUNS: Record<TaggedEntity, [string, string]> = {
  album: ["album", "albums"],
  track: ["track", "tracks"],
  artist: ["artist", "artists"],
  work: ["work", "works"],
  playlist: ["playlist", "playlists"],
};

export function itemsLabel(entityType: TaggedEntity, count: number) {
  const [one, many] = NOUNS[entityType];
  return `${count} ${count === 1 ? one : many}`;
}

function TriStateBox({ state }: { state: TagCheckState }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "grid h-[18px] w-[18px] shrink-0 place-items-center rounded-[5px] border transition-colors",
        state === "unchecked"
          ? "border-zinc-600 bg-transparent"
          : "border-white bg-white text-black",
      )}
    >
      {state === "checked" && <Check className="h-3.5 w-3.5" strokeWidth={3} />}
      {state === "mixed" && <Minus className="h-3.5 w-3.5" strokeWidth={3} />}
    </span>
  );
}

/**
 * Roon's Manage Tags: tick tags for one or many items. A tag only some of
 * the items carry shows as mixed and is left alone unless changed.
 */
export default function ManageTagsDialog({
  entityIds,
  entityType,
  onOpenChange,
  open,
  title,
}: {
  entityIds: string[];
  entityType: TaggedEntity;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  /** What is being tagged, e.g. an album title. */
  title?: string;
}) {
  const queryClient = useQueryClient();
  const ids = useMemo(
    () => Array.from(new Set(entityIds.filter(Boolean))),
    [entityIds],
  );
  const tags = useQuery({
    queryKey: TAGS_KEY,
    queryFn: getTags,
    enabled: open,
  });
  const coverage = useQuery({
    queryKey: [...TAGS_KEY, "coverage", entityType, ids],
    queryFn: () => getTagCoverage(entityType, ids),
    enabled: open && ids.length > 0,
  });
  const initial = useMemo(() => {
    const states = new Map<number, TagCheckState>();
    for (const tag of tags.data ?? []) {
      states.set(
        tag.id,
        tagCheckState(coverage.data?.get(tag.id) ?? 0, ids.length),
      );
    }
    return states;
  }, [coverage.data, ids.length, tags.data]);
  const [edits, setEdits] = useState<Map<number, TagCheckState>>(new Map());
  const [search, setSearch] = useState("");
  const desired = useMemo(() => {
    const next = new Map(initial);
    for (const [id, state] of edits) next.set(id, state);
    return next;
  }, [edits, initial]);

  const manual = (tags.data ?? []).filter((tag) => tag.kind !== "smart");
  const smart = (tags.data ?? []).filter((tag) => tag.kind === "smart");
  const needle = search.trim().toLowerCase();
  const visible = manual.filter(
    (tag) => !needle || tag.name.toLowerCase().includes(needle),
  );
  const exact = manual.some((tag) => tag.name.toLowerCase() === needle);
  const changes = tagChanges(initial, desired);
  const dirty = changes.add.length > 0 || changes.remove.length > 0;

  const toggle = (tag: UserTag) => {
    const current = desired.get(tag.id) ?? "unchecked";
    const before = initial.get(tag.id) ?? "unchecked";
    // Mixed cycles to all, then none, then back to how it was.
    const next: TagCheckState =
      current === "unchecked"
        ? before === "mixed"
          ? "mixed"
          : "checked"
        : current === "mixed"
          ? "checked"
          : "unchecked";
    setEdits((edits) => new Map(edits).set(tag.id, next));
  };

  const create = useMutation({
    mutationFn: (name: string) => createTag(name),
    onSuccess: async (tag) => {
      setSearch("");
      await queryClient.invalidateQueries({ queryKey: TAGS_KEY });
      setEdits((edits) => new Map(edits).set(tag.id, "checked"));
    },
    onError: () => toast("Could not create the tag. Tag names must be unique."),
  });

  const save = useMutation({
    mutationFn: () => bulkTag(entityType, ids, changes),
    onSuccess: async () => {
      const subject = title ? `“${title}”` : itemsLabel(entityType, ids.length);
      toast.success(`Tags updated for ${subject}`);
      setEdits(new Map());
      onOpenChange(false);
      await queryClient.invalidateQueries({ queryKey: TAGS_KEY });
      await queryClient.invalidateQueries({ queryKey: ["library", "focus"] });
    },
    onError: () => toast("Could not save the tags."),
  });

  const submitNew = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (needle && !exact) create.mutate(search.trim());
  };

  const loading = tags.isPending || (ids.length > 0 && coverage.isPending);

  return (
    <Dialog
      onOpenChange={(next) => {
        if (!next) setEdits(new Map());
        onOpenChange(next);
      }}
      open={open}
    >
      <DialogContent className="max-w-md gap-0 p-0">
        <DialogHeader className="border-b border-white/8 px-5 pb-4 pt-5">
          <DialogTitle>Manage tags</DialogTitle>
          <DialogDescription className="truncate">
            {title ?? itemsLabel(entityType, ids.length)}
            {ids.length > 1 && title
              ? ` · ${itemsLabel(entityType, ids.length)}`
              : ""}
          </DialogDescription>
        </DialogHeader>
        <form className="px-5 pt-4" onSubmit={submitNew}>
          <label className="flex h-10 items-center gap-2 rounded-full border border-white/12 bg-white/4 px-3.5 text-sm focus-within:border-white/30">
            <Search className="h-4 w-4 text-zinc-500" />
            <input
              aria-label="Find or create a tag"
              autoFocus
              className="min-w-0 flex-1 bg-transparent text-white outline-none placeholder:text-zinc-500"
              maxLength={100}
              onChange={(event) => setSearch(event.target.value)}
              placeholder="Find or create a tag"
              value={search}
            />
          </label>
        </form>
        <div className="max-h-[46vh] min-h-40 overflow-y-auto px-3 py-3">
          {loading ? (
            <div className="flex h-32 items-center justify-center text-zinc-500">
              <Loader2 className="h-5 w-5 animate-spin" />
            </div>
          ) : (
            <ul className="space-y-0.5">
              {needle && !exact && (
                <li>
                  <button
                    className="flex w-full items-center gap-3 rounded-lg px-2.5 py-2 text-left text-sm text-white hover:bg-white/6 disabled:opacity-60"
                    disabled={create.isPending}
                    onClick={() => create.mutate(search.trim())}
                    type="button"
                  >
                    {create.isPending ? (
                      <Loader2 className="h-[18px] w-[18px] animate-spin" />
                    ) : (
                      <Plus className="h-[18px] w-[18px] text-zinc-400" />
                    )}
                    New tag{" "}
                    <span className="font-semibold">#{search.trim()}</span>
                  </button>
                </li>
              )}
              {visible.map((tag) => {
                const state = desired.get(tag.id) ?? "unchecked";
                const changed = state !== (initial.get(tag.id) ?? "unchecked");
                return (
                  <li key={tag.id}>
                    <button
                      aria-checked={
                        state === "mixed" ? "mixed" : state === "checked"
                      }
                      className="flex w-full items-center gap-3 rounded-lg px-2.5 py-2 text-left text-sm hover:bg-white/6"
                      onClick={() => toggle(tag)}
                      role="checkbox"
                      type="button"
                    >
                      <TriStateBox state={state} />
                      <span
                        aria-hidden="true"
                        className={cn(
                          "h-2 w-2 shrink-0 rounded-full",
                          tagTone(tag).dot,
                        )}
                      />
                      <span className="min-w-0 flex-1 truncate text-zinc-100">
                        {tag.name}
                      </span>
                      <span className="text-xs tabular-nums text-zinc-500">
                        {changed
                          ? state === "checked"
                            ? "add"
                            : state === "unchecked"
                              ? "remove"
                              : "keep"
                          : state === "mixed"
                            ? `${coverage.data?.get(tag.id) ?? 0} of ${ids.length}`
                            : tag.item_count}
                      </span>
                    </button>
                  </li>
                );
              })}
              {!visible.length && !needle && (
                <li className="px-2.5 py-6 text-center text-sm text-zinc-500">
                  No tags yet. Type a name to create one.
                </li>
              )}
            </ul>
          )}
          {smart.length > 0 && !needle && (
            <div className="mt-3 border-t border-white/8 px-2.5 pt-3">
              <p className="mb-2 flex items-center gap-1.5 text-[0.6875rem] font-semibold uppercase tracking-[0.12em] text-zinc-500">
                <Sparkles className="h-3 w-3" /> Smart tags follow their focus
              </p>
              <div className="flex flex-wrap gap-1.5">
                {smart.map((tag) => (
                  <span
                    className={cn(
                      "rounded-full border px-2.5 py-1 text-xs",
                      tagTone(tag).chip,
                    )}
                    key={tag.id}
                  >
                    #{tag.name}
                  </span>
                ))}
              </div>
            </div>
          )}
        </div>
        <div className="flex items-center justify-end gap-2 border-t border-white/8 px-5 py-4">
          <Button onClick={() => onOpenChange(false)} variant="ghost">
            Cancel
          </Button>
          <Button
            disabled={!dirty || save.isPending || ids.length === 0}
            onClick={() => save.mutate()}
          >
            {save.isPending && <Loader2 className="animate-spin" />}
            Save
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
