"use client";

import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import type { UserNote } from "@parson/music-sdk/types";
import { Loader2, Lock } from "lucide-react";
import { useState } from "react";
import { useSaveNote } from "./use-editorial";

/** A private note, clearly labelled as the listener's own words. */
export function NoteEditor({
  entityType,
  id,
  note,
}: {
  entityType: "album" | "artist";
  id: string;
  note: UserNote | null | undefined;
}) {
  const [draft, setDraft] = useState(note?.body ?? "");
  const [editing, setEditing] = useState(false);
  const save = useSaveNote(entityType, id);

  if (!editing && note) {
    return (
      <div className="space-y-3">
        <p className="whitespace-pre-wrap text-[0.9375rem] leading-7 text-zinc-300">
          {note.body}
        </p>
        <div className="flex items-center gap-3 text-xs text-zinc-500">
          <Lock aria-hidden className="h-3.5 w-3.5" />
          <span>Your private note. Only you can see it.</span>
          <button
            className="font-medium text-zinc-400 hover:text-white"
            onClick={() => {
              setDraft(note.body);
              setEditing(true);
            }}
            type="button"
          >
            Edit
          </button>
        </div>
      </div>
    );
  }

  return (
    <form
      className="space-y-3"
      onSubmit={(event) => {
        event.preventDefault();
        save.mutate(draft, { onSuccess: () => setEditing(false) });
      }}
    >
      <Textarea
        aria-label={`Your note about this ${entityType}`}
        className="min-h-28 resize-y"
        maxLength={20_000}
        onChange={(event) => setDraft(event.target.value)}
        placeholder={
          entityType === "album"
            ? "Your thoughts, the pressing you own, where you first heard it…"
            : "Your thoughts about this artist…"
        }
        value={draft}
      />
      <div className="flex items-center justify-between gap-3">
        <p className="flex items-center gap-2 text-xs text-zinc-500">
          <Lock aria-hidden className="h-3.5 w-3.5" />
          Private to you. Saving an empty note removes it.
        </p>
        <div className="flex gap-2">
          {note && (
            <Button
              onClick={() => {
                setDraft(note.body);
                setEditing(false);
              }}
              size="sm"
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
          )}
          <Button disabled={save.isPending} size="sm" type="submit">
            {save.isPending && <Loader2 className="h-4 w-4 animate-spin" />}
            Save note
          </Button>
        </div>
      </div>
    </form>
  );
}
