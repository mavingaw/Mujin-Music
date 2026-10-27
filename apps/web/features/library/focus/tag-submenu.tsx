"use client";

import { Button } from "@/components/ui/button";
import {
  ContextMenuCheckboxItem,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
} from "@/components/ui/context-menu";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import {
  createTag,
  getAssignedTags,
  getTags,
  tagItems,
  untagItems,
  type TaggedEntity,
  type UserTag,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2, Plus, Tag } from "lucide-react";
import { useState, type FormEvent } from "react";
import { toast } from "sonner";
import { tagTone } from "@/features/library/tags/tag-colors";

export const TAGS_KEY = ["library", "tags"] as const;

/** "Tags" submenu for a song, album or artist context menu. */
export function TagSubmenu({
  entityId,
  entityType,
  onCreateTag,
}: {
  entityId: string;
  entityType: TaggedEntity;
  onCreateTag: () => void;
}) {
  return (
    <ContextMenuSub>
      <ContextMenuSubTrigger>
        <Tag className="h-4 w-4" />
        Tags
      </ContextMenuSubTrigger>
      <ContextMenuSubContent className="w-56">
        <TagSubmenuItems
          entityId={entityId}
          entityType={entityType}
          onCreateTag={onCreateTag}
        />
      </ContextMenuSubContent>
    </ContextMenuSub>
  );
}

/** Mounted only while the submenu is open, so cards stay cheap. */
function TagSubmenuItems({
  entityId,
  entityType,
  onCreateTag,
}: {
  entityId: string;
  entityType: TaggedEntity;
  onCreateTag: () => void;
}) {
  const queryClient = useQueryClient();
  const tags = useQuery({ queryKey: TAGS_KEY, queryFn: getTags });
  const assigned = useQuery({
    queryKey: [...TAGS_KEY, "assigned", entityType, entityId],
    queryFn: () => getAssignedTags(entityType, entityId),
  });
  const toggle = useMutation({
    mutationFn: async ({ tag, add }: { tag: UserTag; add: boolean }) => {
      if (add) await tagItems(tag.id, entityType, [entityId]);
      else await untagItems(tag.id, entityType, [entityId]);
      return { tag, add };
    },
    onSuccess: async ({ tag, add }) => {
      toast.success(add ? `Tagged #${tag.name}` : `Removed #${tag.name}`);
      await queryClient.invalidateQueries({ queryKey: ["library", "tag"] });
      await queryClient.invalidateQueries({ queryKey: TAGS_KEY });
      await queryClient.invalidateQueries({ queryKey: ["library", "focus"] });
    },
    onError: () => toast("Could not update tags."),
  });
  const assignedIds = new Set(assigned.data ?? []);
  // Smart tags follow their focus and hold no hand-tagged items.
  const manualTags = (tags.data ?? []).filter((tag) => tag.kind !== "smart");

  return (
    <>
      {(tags.isPending || assigned.isPending) && (
        <ContextMenuItem disabled>
          <Loader2 className="h-4 w-4 animate-spin" />
          Loading tags…
        </ContextMenuItem>
      )}
      {manualTags.map((tag) => (
        <ContextMenuCheckboxItem
          checked={assignedIds.has(tag.id)}
          disabled={toggle.isPending || assigned.isPending}
          key={tag.id}
          onSelect={(event) => {
            event.preventDefault();
            toggle.mutate({ tag, add: !assignedIds.has(tag.id) });
          }}
        >
          <span
            aria-hidden="true"
            className={`h-2 w-2 shrink-0 rounded-full ${tagTone(tag).dot}`}
          />
          <span className="truncate">{tag.name}</span>
        </ContextMenuCheckboxItem>
      ))}
      {manualTags.length > 0 && <ContextMenuSeparator />}
      <ContextMenuItem onSelect={onCreateTag}>
        <Plus className="h-4 w-4" />
        New tag…
      </ContextMenuItem>
    </>
  );
}

/** Creates a tag and applies it to one entity. */
export function CreateTagDialog({
  entityId,
  entityType,
  onOpenChange,
  open,
}: {
  entityId: string;
  entityType: TaggedEntity;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const queryClient = useQueryClient();
  const [name, setName] = useState("");
  const create = useMutation({
    mutationFn: async (value: string) => {
      const tag = await createTag(value);
      await tagItems(tag.id, entityType, [entityId]);
      return tag;
    },
    onSuccess: async (tag) => {
      toast.success(`Tagged #${tag.name}`);
      setName("");
      onOpenChange(false);
      await queryClient.invalidateQueries({ queryKey: TAGS_KEY });
      await queryClient.invalidateQueries({ queryKey: ["library", "focus"] });
    },
    onError: () => toast("Could not create the tag. Tag names must be unique."),
  });
  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (name.trim()) create.mutate(name.trim());
  };
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent aria-describedby={undefined}>
        <DialogHeader>
          <DialogTitle>New tag</DialogTitle>
        </DialogHeader>
        <form className="grid gap-5" onSubmit={submit}>
          <Field>
            <FieldLabel htmlFor="new-tag-name">Name</FieldLabel>
            <Input
              autoFocus
              id="new-tag-name"
              maxLength={100}
              onChange={(event) => setName(event.target.value)}
              placeholder="Late night"
              value={name}
            />
          </Field>
          <Button disabled={!name.trim() || create.isPending} type="submit">
            {create.isPending ? "Creating…" : "Create and tag"}
          </Button>
        </form>
      </DialogContent>
    </Dialog>
  );
}
