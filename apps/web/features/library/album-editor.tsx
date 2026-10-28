"use client";

import { Button } from "@/components/ui/button";
import { ContextMenuItem } from "@/components/ui/context-menu";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { DropdownMenuItem } from "@/components/ui/dropdown-menu";
import {
  editAlbumMetadata,
  getAlbumInfo,
  sourcesPatch,
  updateAlbumSources,
  uploadAlbumCover,
  type LibraryAlbum,
  type SourcesDraft,
} from "@parson/music-sdk";
import type { LibraryMetadataPatch } from "@parson/music-sdk/types";
import { useQueryClient } from "@tanstack/react-query";
import { Loader2, Pencil } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import {
  changedMetadata,
  hasMetadataChanges,
  validateMetadata,
} from "./metadata-state";
import { MetadataFields, type MetadataUpdater } from "./metadata-fields";
import { AlbumSourcesPanel } from "./album-sources-panel";
import { useAlbumAbout } from "./credits/use-editorial";
import { toast } from "sonner";

type Trigger = "context" | "dropdown" | "none";
export type AlbumEditorTab = "details" | "sources";

const EMPTY_DRAFT: SourcesDraft = { sources: {}, edits: {} };

function editableAlbum(album: LibraryAlbum): LibraryMetadataPatch {
  return {
    album: {
      name: album.name,
      cover_url: album.cover_url,
      first_release_date: album.first_release_date,
      primary_type: album.primary_type,
      description: album.description,
    },
    artist: {
      name: album.artist_object.name,
      icon_url: album.artist_object.icon_url,
      description: album.artist_object.description,
    },
  };
}

export default function AlbumEditor({
  albumId,
  label = "Edit album metadata",
  trigger = "dropdown",
  open: controlledOpen,
  onOpenChange,
  initialTab = "details",
  onEditCredits,
}: {
  albumId: string;
  label?: string;
  trigger?: Trigger;
  /** Controlled open state (with `trigger="none"`). */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  initialTab?: AlbumEditorTab;
  /** Opens the album's credits editor from the Metadata preference tab. */
  onEditCredits?: () => void;
}) {
  const queryClient = useQueryClient();
  const [uncontrolledOpen, setUncontrolledOpen] = useState(false);
  const open = controlledOpen ?? uncontrolledOpen;
  const setOpen = (next: boolean) => {
    setUncontrolledOpen(next);
    onOpenChange?.(next);
  };
  const [tab, setTab] = useState<AlbumEditorTab>(initialTab);
  const [draft, setDraft] = useState<SourcesDraft>(EMPTY_DRAFT);
  const aboutQuery = useAlbumAbout(open ? albumId : null);
  const sources = aboutQuery.data?.sources;
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [coverFile, setCoverFile] = useState<File | null>(null);
  const [error, setError] = useState("");
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [patch, setPatch] = useState<LibraryMetadataPatch>({
    album: {},
    artist: {},
  });
  const original = useRef<LibraryMetadataPatch | null>(null);

  useEffect(() => {
    if (!error) return;
    toast(error, { id: "album-metadata-error" });
    setError("");
  }, [error]);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    original.current = null;
    setTab(initialTab);
    setDraft(EMPTY_DRAFT);
    setLoading(true);
    setError("");
    setShowAdvanced(false);
    setCoverFile(null);
    getAlbumInfo(albumId, false)
      .then((album) => {
        if (cancelled) return;
        const value = editableAlbum(album as LibraryAlbum);
        original.current = value;
        setPatch(value);
      })
      .catch(() => !cancelled && setError("Could not load album metadata."))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
    // The tab to open on is read when the dialog opens.
  }, [albumId, open, initialTab]);

  const update: MetadataUpdater = (section, field, value) =>
    setPatch((current) => ({
      ...current,
      [section]: { ...current[section], [field]: value },
    }));

  const save = async () => {
    if (!original.current || saving) return;
    let changes = changedMetadata(original.current, patch);
    const validationError = validateMetadata(changes);
    if (validationError) return setError(validationError);
    const metadataChanged = hasMetadataChanges(changes);
    const sourceChanges = sources ? sourcesPatch(sources.facts, draft) : null;
    if (!metadataChanged && !coverFile && !sourceChanges) return setOpen(false);
    setSaving(true);
    setError("");
    try {
      const response = metadataChanged
        ? await editAlbumMetadata(albumId, {
            album: changes.album,
            artist: changes.artist,
          })
        : null;
      if (coverFile) await uploadAlbumCover(albumId, coverFile);
      if (sourceChanges) await updateAlbumSources(albumId, sourceChanges);
      if (response && !coverFile) {
        queryClient.setQueryData(["albums", albumId], response.album);
        queryClient.setQueryData(
          ["artists", response.artist.id],
          response.artist,
        );
      }
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: ["albums", albumId],
        }),
        queryClient.invalidateQueries({
          queryKey: ["album-about", albumId],
        }),
        queryClient.invalidateQueries({
          queryKey: ["library"],
          refetchType: "none",
        }),
        queryClient.invalidateQueries({
          queryKey: ["home"],
          refetchType: "none",
        }),
        queryClient.invalidateQueries({
          queryKey: ["search"],
          refetchType: "none",
        }),
      ]);
      setOpen(false);
    } catch (cause) {
      const message =
        (cause as { response?: { data?: { message?: string } } })?.response
          ?.data?.message ?? null;
      setError(message ?? "Could not save album metadata.");
    } finally {
      setSaving(false);
    }
  };

  const openEditor = (event: Event) => {
    event.preventDefault();
    setOpen(true);
  };
  const triggerNode =
    trigger === "context" ? (
      <ContextMenuItem onSelect={openEditor}>
        <Pencil /> {label}
      </ContextMenuItem>
    ) : trigger === "dropdown" ? (
      <DropdownMenuItem onSelect={openEditor}>
        <Pencil /> {label}
      </DropdownMenuItem>
    ) : null;

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      {triggerNode}
      <DialogContent className="grid-rows-[auto_auto_minmax(0,1fr)] gap-4 p-6 sm:max-w-2xl">
        <DialogHeader className="pr-10">
          <DialogTitle className="text-base">Edit album</DialogTitle>
        </DialogHeader>
        <div
          aria-label="Album editor sections"
          className="flex gap-5 border-b border-white/10"
          role="tablist"
        >
          {(
            [
              ["details", "Details"],
              ["sources", "Metadata preference"],
            ] as const
          ).map(([key, title]) => (
            <button
              aria-selected={tab === key}
              className={`-mb-px border-b-2 pb-2.5 text-[0.8125rem] font-semibold transition-colors ${
                tab === key
                  ? "border-white text-white"
                  : "border-transparent text-zinc-500 hover:text-zinc-200"
              }`}
              key={key}
              onClick={() => setTab(key)}
              role="tab"
              type="button"
            >
              {title}
            </button>
          ))}
        </div>
        <form
          className="grid min-h-0 grid-rows-[minmax(0,1fr)_auto] gap-6"
          onSubmit={(event) => {
            event.preventDefault();
            void save();
          }}
        >
          <div className="min-h-0 overflow-y-auto pr-1">
            {tab === "sources" ? (
              sources ? (
                <AlbumSourcesPanel
                  draft={draft}
                  onDraftChange={setDraft}
                  onEditCredits={
                    onEditCredits
                      ? () => {
                          setOpen(false);
                          onEditCredits();
                        }
                      : undefined
                  }
                  sources={sources}
                />
              ) : (
                <div className="flex items-center gap-2 py-10 text-sm text-zinc-500">
                  {aboutQuery.isError ? (
                    "This server cannot choose metadata sources yet."
                  ) : (
                    <>
                      <Loader2 className="size-4 animate-spin" /> Loading
                      sources
                    </>
                  )}
                </div>
              )
            ) : loading ? (
              <div className="flex items-center gap-2 py-10 text-sm text-zinc-500">
                <Loader2 className="size-4 animate-spin" /> Loading album
                metadata
              </div>
            ) : (
              <MetadataFields
                coverFile={coverFile}
                mode="album"
                onCoverSelect={setCoverFile}
                patch={patch}
                showAdvanced={showAdvanced}
                toggleAdvanced={() => setShowAdvanced((value) => !value)}
                update={update}
              />
            )}
          </div>
          <DialogFooter className="flex-row items-center justify-end gap-2">
            <Button
              type="button"
              variant="ghost"
              onClick={() => setOpen(false)}
            >
              Cancel
            </Button>
            <Button
              disabled={loading || saving || !original.current}
              type="submit"
            >
              {saving && <Loader2 className="size-4 animate-spin" />}
              {saving ? "Saving" : "Save changes"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
