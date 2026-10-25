"use client";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Field, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import {
  deleteSavedFocus,
  encodeFocus,
  focusSummary,
  getSavedFocuses,
  reorderSavedFocuses,
  saveFocus,
  updateSavedFocus,
  type FocusQuery,
  type SavedFocus,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowDown,
  ArrowUp,
  Bookmark,
  Feather,
  BookmarkCheck,
  BookmarkPlus,
  Disc3,
  Music2,
  Settings2,
  Trash2,
  UserRound,
} from "lucide-react";
import { useState, type FormEvent } from "react";
import { toast } from "sonner";

const SAVED_KEY = ["library", "focus", "saved"] as const;

const VIEW_ICONS = {
  albums: Disc3,
  songs: Music2,
  artists: UserRound,
  composers: Feather,
} as const;

function sameFocus(left: FocusQuery, right: FocusQuery) {
  return (
    left.entity === right.entity && encodeFocus(left) === encodeFocus(right)
  );
}

export default function SavedFocusMenu({
  query,
  setQuery,
}: {
  query: FocusQuery;
  setQuery: (query: FocusQuery) => void;
}) {
  const [saveOpen, setSaveOpen] = useState(false);
  const [manageOpen, setManageOpen] = useState(false);
  const saved = useQuery({ queryKey: SAVED_KEY, queryFn: getSavedFocuses });
  const active = saved.data?.find((item) => sameFocus(item.query, query));

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            aria-label="Bookmarks"
            className="grid h-9 w-9 place-items-center rounded-full text-zinc-400 hover:bg-white/8 hover:text-white data-[state=open]:bg-white/8 data-[state=open]:text-white"
            title={active ? `Bookmarked as ${active.name}` : "Bookmarks"}
            type="button"
          >
            {active ? (
              <BookmarkCheck className="h-4 w-4 text-white" />
            ) : (
              <Bookmark className="h-4 w-4" />
            )}
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-72">
          <DropdownMenuItem
            disabled={Boolean(active)}
            onSelect={() => setSaveOpen(true)}
          >
            <BookmarkPlus />
            {active ? `Saved as “${active.name}”` : "Bookmark this view…"}
          </DropdownMenuItem>
          {Boolean(saved.data?.length) && (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuLabel>Bookmarks</DropdownMenuLabel>
              {saved.data?.map((item) => {
                const Icon = VIEW_ICONS[item.view] ?? Disc3;
                return (
                  <DropdownMenuItem
                    className="items-start"
                    key={item.id}
                    onSelect={() => setQuery(item.query)}
                  >
                    <Icon className="mt-0.5" />
                    <span className="min-w-0">
                      <span className="block truncate text-zinc-100">
                        {item.name}
                      </span>
                      {focusSummary(item.query) && (
                        <span className="block truncate text-xs text-zinc-500">
                          {focusSummary(item.query)}
                        </span>
                      )}
                    </span>
                  </DropdownMenuItem>
                );
              })}
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => setManageOpen(true)}>
                <Settings2 />
                Manage bookmarks
              </DropdownMenuItem>
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
      <SaveFocusDialog
        onOpenChange={setSaveOpen}
        open={saveOpen}
        query={query}
      />
      <ManageFocusesDialog
        onOpenChange={setManageOpen}
        open={manageOpen}
        saved={saved.data ?? []}
      />
    </>
  );
}

function SaveFocusDialog({
  onOpenChange,
  open,
  query,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
  query: FocusQuery;
}) {
  const queryClient = useQueryClient();
  const [name, setName] = useState("");
  const save = useMutation({
    mutationFn: (value: string) => saveFocus(value, query),
    onSuccess: (saved) => {
      queryClient.setQueryData<SavedFocus[]>(SAVED_KEY, (current) => [
        ...(current ?? []),
        saved,
      ]);
      toast.success(`Bookmarked ${saved.name}`);
      setName("");
      onOpenChange(false);
    },
    onError: () => toast("Could not save this bookmark."),
  });
  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (name.trim()) save.mutate(name.trim());
  };
  const summary = focusSummary(query);

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Bookmark this view</DialogTitle>
          <DialogDescription>
            Saves the {query.entity} browser with its focus and sort
            {summary ? `: ${summary}` : "."}
          </DialogDescription>
        </DialogHeader>
        <form className="grid gap-5" onSubmit={submit}>
          <Field>
            <FieldLabel htmlFor="saved-focus-name">Name</FieldLabel>
            <Input
              autoFocus
              id="saved-focus-name"
              maxLength={100}
              onChange={(event) => setName(event.target.value)}
              placeholder="Albums I've never listened to"
              value={name}
            />
          </Field>
          <Button disabled={!name.trim() || save.isPending} type="submit">
            {save.isPending ? "Saving…" : "Save bookmark"}
          </Button>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function ManageFocusesDialog({
  onOpenChange,
  open,
  saved,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
  saved: SavedFocus[];
}) {
  const queryClient = useQueryClient();
  const refresh = () => queryClient.invalidateQueries({ queryKey: SAVED_KEY });
  const reorder = useMutation({
    mutationFn: reorderSavedFocuses,
    onSuccess: (items) => queryClient.setQueryData(SAVED_KEY, items),
    onError: () => toast("Could not reorder bookmarks."),
  });
  const rename = useMutation({
    mutationFn: ({ id, name }: { id: number; name: string }) =>
      updateSavedFocus(id, { name }),
    onSuccess: () => void refresh(),
    onError: () => toast("Could not rename the bookmark."),
  });
  const remove = useMutation({
    mutationFn: deleteSavedFocus,
    onSuccess: () => void refresh(),
    onError: () => toast("Could not delete the bookmark."),
  });

  const move = (index: number, offset: -1 | 1) => {
    const ids = saved.map((item) => item.id);
    const target = index + offset;
    if (target < 0 || target >= ids.length) return;
    [ids[index], ids[target]] = [ids[target]!, ids[index]!];
    reorder.mutate(ids);
  };

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent aria-describedby={undefined}>
        <DialogHeader>
          <DialogTitle>Bookmarks</DialogTitle>
        </DialogHeader>
        {saved.length === 0 ? (
          <p className="text-sm text-zinc-500">No bookmarks yet.</p>
        ) : (
          <ul className="grid gap-2">
            {saved.map((item, index) => (
              <li
                className="flex items-center gap-2 rounded-lg border border-white/8 px-2 py-1.5"
                key={item.id}
              >
                <input
                  aria-label={`Rename ${item.name}`}
                  className="min-w-0 flex-1 rounded bg-transparent px-1.5 py-1 text-sm text-zinc-100 outline-none focus:bg-white/5"
                  defaultValue={item.name}
                  maxLength={100}
                  onBlur={(event) => {
                    const next = event.target.value.trim();
                    if (next && next !== item.name) {
                      rename.mutate({ id: item.id, name: next });
                    } else {
                      event.target.value = item.name;
                    }
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") event.currentTarget.blur();
                  }}
                />
                <button
                  aria-label={`Move ${item.name} up`}
                  className="grid h-8 w-8 place-items-center rounded text-zinc-500 hover:bg-white/8 hover:text-white disabled:opacity-30"
                  disabled={index === 0 || reorder.isPending}
                  onClick={() => move(index, -1)}
                  type="button"
                >
                  <ArrowUp className="h-4 w-4" />
                </button>
                <button
                  aria-label={`Move ${item.name} down`}
                  className="grid h-8 w-8 place-items-center rounded text-zinc-500 hover:bg-white/8 hover:text-white disabled:opacity-30"
                  disabled={index === saved.length - 1 || reorder.isPending}
                  onClick={() => move(index, 1)}
                  type="button"
                >
                  <ArrowDown className="h-4 w-4" />
                </button>
                <button
                  aria-label={`Delete ${item.name}`}
                  className="grid h-8 w-8 place-items-center rounded text-zinc-500 hover:bg-rose-500/10 hover:text-rose-300"
                  onClick={() => remove.mutate(item.id)}
                  type="button"
                >
                  <Trash2 className="h-4 w-4" />
                </button>
              </li>
            ))}
          </ul>
        )}
      </DialogContent>
    </Dialog>
  );
}
