"use client";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/cn";
import {
  TAG_COLORS,
  createTag,
  criterionLabel,
  entityNoun,
  updateTag,
  type FocusQuery,
  type TagColor,
  type UserTag,
} from "@parson/music-sdk";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Loader2, Sparkles } from "lucide-react";
import { useState, type FormEvent } from "react";
import { toast } from "sonner";
import { TAGS_KEY } from "@/features/library/focus/tag-submenu";
import { fallbackTagColor, tagTone } from "./tag-colors";

/** The focus a smart tag follows, as chips. */
export function SmartQuerySummary({ query }: { query: FocusQuery }) {
  const filters = query.filters ?? [];
  return (
    <div className="flex flex-wrap items-center gap-1.5 text-xs">
      <span className="text-zinc-500">
        <span className="capitalize">
          {entityNoun(query.entity ?? "albums", 2)}
        </span>
        {filters.length ? " where" : ", all of them"}
      </span>
      {filters.map((filter, index) => (
        <span
          className={cn(
            "rounded-full border px-2 py-0.5",
            filter.exclude
              ? "border-rose-400/30 bg-rose-500/10 text-rose-100"
              : "border-white/12 bg-white/6 text-zinc-100",
          )}
          key={index}
        >
          {criterionLabel(filter)}
        </span>
      ))}
      {query.text?.trim() && (
        <span className="rounded-full border border-white/12 bg-white/6 px-2 py-0.5 text-zinc-100">
          “{query.text.trim()}”
        </span>
      )}
    </div>
  );
}

/**
 * Creates or edits a tag: name, colour and a note. Given a focus, the new
 * tag is smart and follows it.
 */
export default function TagEditorDialog({
  onOpenChange,
  onSaved,
  open,
  query,
  tag,
}: {
  onOpenChange: (open: boolean) => void;
  onSaved?: (tag: UserTag) => void;
  open: boolean;
  /** Creates a smart tag that follows this focus. */
  query?: FocusQuery;
  /** Edits this tag instead of creating one. */
  tag?: UserTag;
}) {
  const queryClient = useQueryClient();
  const [name, setName] = useState(tag?.name ?? "");
  const [color, setColor] = useState<TagColor | null>(tag?.color ?? null);
  const [description, setDescription] = useState(tag?.description ?? "");
  const smartQuery = tag?.query ?? query;
  const shownColor = color ?? fallbackTagColor(name || "tag");
  const save = useMutation({
    mutationFn: () =>
      tag
        ? updateTag(tag.id, {
            name,
            color,
            description: description.trim() || null,
          })
        : createTag(name, {
            color,
            description: description.trim() || null,
            query: query ?? null,
          }),
    onSuccess: async (saved) => {
      toast.success(
        tag
          ? `Saved #${saved.name}`
          : query
            ? `Smart tag #${saved.name} created`
            : `Created #${saved.name}`,
      );
      onOpenChange(false);
      onSaved?.(saved);
      await queryClient.invalidateQueries({ queryKey: TAGS_KEY });
      await queryClient.invalidateQueries({ queryKey: ["library", "focus"] });
    },
    onError: () => toast("Could not save the tag. Tag names must be unique."),
  });
  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (name.trim()) save.mutate();
  };
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {smartQuery && <Sparkles className="h-4 w-4 text-violet-300" />}
            {tag ? "Edit tag" : query ? "New smart tag" : "New tag"}
          </DialogTitle>
          <DialogDescription>
            {smartQuery
              ? "A smart tag holds whatever this focus matches, and keeps up as your library changes."
              : "Tag albums, artists, tracks, works and playlists from any menu."}
          </DialogDescription>
        </DialogHeader>
        {smartQuery && (
          <div className="rounded-xl border border-violet-300/15 bg-violet-400/6 p-3">
            <SmartQuerySummary query={smartQuery} />
          </div>
        )}
        <form className="grid gap-5" onSubmit={submit}>
          <Field>
            <FieldLabel htmlFor="tag-name">Name</FieldLabel>
            <Input
              autoFocus
              id="tag-name"
              maxLength={100}
              onChange={(event) => setName(event.target.value)}
              placeholder={query ? "Hi-res jazz" : "Late night"}
              value={name}
            />
          </Field>
          <fieldset>
            <legend className="mb-2 text-sm font-medium text-zinc-200">
              Colour
            </legend>
            <div className="flex flex-wrap gap-2">
              {TAG_COLORS.map((option) => (
                <button
                  aria-label={option}
                  aria-pressed={shownColor === option}
                  className={cn(
                    "grid h-7 w-7 place-items-center rounded-full ring-offset-2 ring-offset-zinc-950 transition-transform hover:scale-110",
                    shownColor === option && "ring-2 ring-white",
                  )}
                  key={option}
                  onClick={() => setColor(option)}
                  type="button"
                >
                  <span
                    className={cn(
                      "h-5 w-5 rounded-full",
                      tagTone({ name: option, color: option }).dot,
                    )}
                  />
                </button>
              ))}
            </div>
          </fieldset>
          <Field>
            <FieldLabel htmlFor="tag-description">Note</FieldLabel>
            <Textarea
              id="tag-description"
              maxLength={500}
              onChange={(event) => setDescription(event.target.value)}
              placeholder="What this tag is for (optional)"
              rows={2}
              value={description}
            />
          </Field>
          <div className="flex justify-end gap-2">
            <Button onClick={() => onOpenChange(false)} variant="ghost">
              Cancel
            </Button>
            <Button disabled={!name.trim() || save.isPending} type="submit">
              {save.isPending && <Loader2 className="animate-spin" />}
              {tag ? "Save" : "Create"}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
