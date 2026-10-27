"use client";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  getFocusPage,
  type FocusEntity,
  type FocusQuery,
  type TaggedEntity,
} from "@parson/music-sdk";
import { Loader2, Sparkles, Tag, Tags } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import ManageTagsDialog from "./manage-tags-dialog";
import TagEditorDialog from "./tag-editor-dialog";

const TAGGABLE: Partial<Record<FocusEntity, TaggedEntity>> = {
  albums: "album",
  songs: "track",
  artists: "artist",
};

/** Most items "Tag all" applies to at once. */
const MAX_BULK = 2_000;
const PAGE = 200;

async function focusIds(query: FocusQuery): Promise<string[]> {
  const ids: string[] = [];
  for (let offset = 0; offset < MAX_BULK; offset += PAGE) {
    const page = await getFocusPage(query, offset, PAGE);
    const rows =
      query.entity === "songs"
        ? page.songs
        : query.entity === "artists"
          ? page.artists
          : page.albums;
    ids.push(...rows.map((row) => row.id));
    if (rows.length < PAGE || ids.length >= page.total) break;
  }
  return ids;
}

/**
 * Tag actions for a Library focus: keep the focus as a smart tag, or tag
 * everything it shows in one go.
 */
export default function FocusTagMenu({ query }: { query: FocusQuery }) {
  const entity = query.entity ?? "albums";
  const tagged = TAGGABLE[entity];
  const focused = query.filters.length > 0 || Boolean(query.text?.trim());
  const [smartOpen, setSmartOpen] = useState(false);
  const [bulkIds, setBulkIds] = useState<string[] | null>(null);
  const [loading, setLoading] = useState(false);
  if (!tagged) return null;

  const tagAll = async () => {
    setLoading(true);
    try {
      const ids = await focusIds({ ...query, entity });
      if (!ids.length) toast("Nothing to tag in this view.");
      else setBulkIds(ids);
    } catch {
      toast("Could not load the items to tag.");
    } finally {
      setLoading(false);
    }
  };

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            aria-label="Tag"
            className="flex h-9 items-center gap-1.5 rounded-full px-2 text-sm text-zinc-300 hover:text-white"
            type="button"
          >
            {loading ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Tag className="h-4 w-4" />
            )}
            <span className="hidden md:inline">Tag</span>
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-64">
          {focused && (
            <DropdownMenuItem onSelect={() => setSmartOpen(true)}>
              <Sparkles className="text-violet-300" />
              Save focus as smart tag…
            </DropdownMenuItem>
          )}
          <DropdownMenuItem onSelect={() => void tagAll()}>
            <Tags />
            {focused ? "Tag everything shown…" : `Tag all ${entity}…`}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      {smartOpen && (
        <TagEditorDialog
          onOpenChange={setSmartOpen}
          open={smartOpen}
          query={{ ...query, entity, seed: undefined }}
        />
      )}
      {bulkIds && (
        <ManageTagsDialog
          entityIds={bulkIds}
          entityType={tagged}
          onOpenChange={(open) => !open && setBulkIds(null)}
          open
        />
      )}
    </>
  );
}
