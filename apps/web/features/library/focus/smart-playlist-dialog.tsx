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
import {
  createSmartPlaylist,
  focusSummary,
  type FocusQuery,
} from "@parson/music-sdk";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useRouter } from "next/navigation";
import { useEffect, useState, type FormEvent } from "react";
import { toast } from "sonner";

export default function SmartPlaylistDialog({
  onOpenChange,
  open,
  query,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
  query: FocusQuery;
}) {
  const router = useRouter();
  const queryClient = useQueryClient();
  const summary = focusSummary(query);
  const [name, setName] = useState("");
  useEffect(() => {
    if (open) setName(summary.slice(0, 200));
  }, [open, summary]);

  const create = useMutation({
    mutationFn: (value: string) =>
      createSmartPlaylist(value, query, { description: summary || undefined }),
    onSuccess: async (playlist) => {
      await queryClient.invalidateQueries({ queryKey: ["playlists"] });
      toast.success(
        `Created ${playlist.name} with ${playlist.song_count} song${playlist.song_count === 1 ? "" : "s"}`,
      );
      onOpenChange(false);
      router.push(`/playlist?id=${playlist.id}`);
    },
    onError: () => toast("Could not create the smart playlist."),
  });

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (name.trim()) create.mutate(name.trim());
  };

  return (
    <Dialog
      onOpenChange={(next) => {
        if (!create.isPending) onOpenChange(next);
      }}
      open={open}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create smart playlist</DialogTitle>
          <DialogDescription>
            Every song that matches this focus
            {summary ? ` (${summary})` : ""}. Refresh the playlist any time to
            pick up new music.
          </DialogDescription>
        </DialogHeader>
        <form className="grid gap-5" onSubmit={submit}>
          <Field>
            <FieldLabel htmlFor="smart-playlist-name">Name</FieldLabel>
            <Input
              autoFocus
              id="smart-playlist-name"
              maxLength={200}
              onChange={(event) => setName(event.target.value)}
              value={name}
            />
          </Field>
          <Button disabled={!name.trim() || create.isPending} type="submit">
            {create.isPending ? "Creating…" : "Create smart playlist"}
          </Button>
        </form>
      </DialogContent>
    </Dialog>
  );
}
