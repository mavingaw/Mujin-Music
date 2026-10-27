"use client";

import { Button } from "@/components/ui/button";
import { invalidateCatalogRevisionQueries } from "@/features/library/library-readiness-state";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { hasLocalDesktopBridge, selectMusicFolder } from "@/lib/desktop/bridge";
import {
  getLibraryCatalog,
  getLibraryReadiness,
  getLibraryRoots,
  type LibraryRoot,
  indexLibrary,
  refreshCurrentLibrary,
  relocateLibraryRoot,
  removeLibraryRoot,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronRight, Folder, Loader2, Trash2, Wrench } from "lucide-react";
import Link from "next/link";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { LibraryFolderBrowser } from "./library-folder-browser";
import { locateStartFolders } from "./locate-folder";

export default function LibrarySettings() {
  const queryClient = useQueryClient();
  const [desktop, setDesktop] = useState(false);
  const [addOpen, setAddOpen] = useState(false);
  const [removePath, setRemovePath] = useState<string | null>(null);
  const [locateRoot, setLocateRoot] = useState<LibraryRoot | null>(null);
  const locatePath = locateRoot?.path ?? null;
  const [busy, setBusy] = useState<
    "add" | "refresh" | "remove" | "locate" | null
  >(null);
  const roots = useQuery({
    queryKey: ["library-roots"],
    queryFn: getLibraryRoots,
  });
  const readiness = useQuery({
    queryKey: ["library-readiness", "settings"],
    queryFn: getLibraryReadiness,
  });
  const catalog = useQuery({
    queryKey: ["library-catalog", "settings-count"],
    queryFn: () => getLibraryCatalog(0, 1),
  });

  useEffect(() => setDesktop(hasLocalDesktopBridge()), []);

  async function addLibrary(nextPath: string) {
    const selected = nextPath.trim();
    if (!selected || busy) return;
    setBusy("add");
    try {
      await indexLibrary(selected);
      setAddOpen(false);
      await Promise.all([
        roots.refetch(),
        readiness.refetch(),
        catalog.refetch(),
      ]);
      toast.success("Music folder added.");
    } catch {
      toast("Could not use that folder. Check the path and try again.");
    } finally {
      setBusy(null);
    }
  }

  /**
   * A folder from another computer (a restored backup) is missing here: the
   * server indexes its new location and moves playlists, favourites,
   * history and edits over to it, track by track, before dropping the old
   * path.
   */
  async function relocate(oldPath: string, nextPath: string) {
    const selected = nextPath.trim();
    if (!selected || busy) return;
    setBusy("locate");
    try {
      const report = await relocateLibraryRoot(oldPath, selected);
      setLocateRoot(null);
      await Promise.all([
        roots.refetch(),
        readiness.refetch(),
        catalog.refetch(),
      ]);
      await invalidateCatalogRevisionQueries(queryClient);
      toast.success(
        report.missing_tracks
          ? `Found ${report.matched_tracks.toLocaleString()} tracks; ${report.missing_tracks.toLocaleString()} weren’t in that folder. Playlists and history moved with the rest.`
          : `Found all ${report.matched_tracks.toLocaleString()} tracks. Playlists, favourites and history moved with them.`,
      );
    } catch {
      toast("Could not use that folder. Check the path and try again.");
    } finally {
      setBusy(null);
    }
  }

  async function locateFolder(root: LibraryRoot) {
    if (hasLocalDesktopBridge()) {
      // The system picker opens near where the music used to be.
      const selected = await selectMusicFolder(locateStartFolders(root)[0]);
      if (selected) await relocate(root.path, selected);
      return;
    }
    setLocateRoot(root);
  }

  async function changeFolder() {
    if (hasLocalDesktopBridge()) {
      const selected = await selectMusicFolder();
      if (selected) await addLibrary(selected);
      return;
    }
    setAddOpen(true);
  }

  async function removeFolder() {
    if (!removePath || busy) return;
    setBusy("remove");
    try {
      await removeLibraryRoot(removePath);
      setRemovePath(null);
      await Promise.all([
        roots.refetch(),
        readiness.refetch(),
        catalog.refetch(),
      ]);
      await invalidateCatalogRevisionQueries(queryClient);
      toast.success("Music folder removed.");
    } catch {
      toast("Could not remove that folder right now.");
    } finally {
      setBusy(null);
    }
  }

  async function refresh() {
    if (busy) return;
    setBusy("refresh");
    try {
      await refreshCurrentLibrary();
      await Promise.all([readiness.refetch(), catalog.refetch()]);
      await invalidateCatalogRevisionQueries(queryClient);
      toast.success("Library checked for changes.");
    } catch {
      toast("Could not check the library right now.");
    } finally {
      setBusy(null);
    }
  }

  const paths = roots.data ?? [];
  const status = !readiness.data
    ? "Loading…"
    : readiness.data.state === "ready"
      ? "Ready"
      : "Needs attention";

  return (
    <div className="space-y-8">
      <section>
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="text-base font-semibold text-white">
              {paths.length > 1 ? "Music folders" : "Music folder"}
            </h2>
            <p className="mt-1 text-sm text-zinc-500">
              {desktop
                ? "Mujin Music watches this folder for your music."
                : "Folders mounted and available to Mujin Music."}
            </p>
          </div>
          <Button onClick={() => void changeFolder()}>
            {paths.length ? "Add folder" : "Add music folder"}
          </Button>
        </div>
        <div className="mt-5 overflow-hidden rounded-lg border border-white/10">
          {roots.isLoading ? (
            <div className="flex h-20 items-center justify-center text-zinc-500">
              <Loader2
                className="h-4 w-4 animate-spin"
                aria-label="Loading music folders"
              />
            </div>
          ) : paths.length ? (
            paths.map((root) => (
              <div
                className="flex items-center gap-3 border-b border-white/[0.08] px-4 py-4 last:border-0"
                key={root.path}
              >
                <Folder
                  className={`h-5 w-5 shrink-0 ${root.available === false ? "text-amber-300/80" : "text-zinc-500"}`}
                />
                <div className="min-w-0 flex-1">
                  <p
                    className="truncate text-sm text-zinc-200"
                    title={root.path}
                  >
                    {root.path}
                  </p>
                  {root.available === false ? (
                    <p className="mt-0.5 text-xs text-amber-200/80">
                      Not found on this server. Moved here from another
                      computer? Show Mujin Music where this music is now.
                    </p>
                  ) : null}
                </div>
                {root.available === false ? (
                  <Button
                    className="h-8 px-3"
                    disabled={Boolean(busy)}
                    onClick={() => void locateFolder(root)}
                    size="sm"
                  >
                    {busy === "locate" ? "Finding…" : "Locate…"}
                  </Button>
                ) : null}
                <Button
                  aria-label={`Remove ${root.path}`}
                  className="h-8 px-2 text-zinc-400 hover:text-red-300"
                  disabled={Boolean(busy)}
                  onClick={() => setRemovePath(root.path)}
                  variant="ghost"
                >
                  <Trash2 className="h-4 w-4" />
                  <span className="hidden sm:inline">Remove</span>
                </Button>
              </div>
            ))
          ) : (
            <p className="px-4 py-5 text-sm text-zinc-500">
              No music folder is configured.
            </p>
          )}
        </div>
      </section>

      <section className="border-t border-white/[0.08] pt-7">
        <h2 className="text-base font-semibold text-white">Library status</h2>
        <dl className="mt-4 grid gap-4 text-sm sm:grid-cols-2">
          <div>
            <dt className="text-zinc-500">Status</dt>
            <dd className="mt-1 text-zinc-200">{status}</dd>
          </div>
          <div>
            <dt className="text-zinc-500">Tracks</dt>
            <dd className="mt-1 text-zinc-200">
              {catalog.data ? catalog.data.totalSongs.toLocaleString() : "—"}
            </dd>
          </div>
        </dl>
        <Button
          className="mt-6"
          disabled={Boolean(busy) || !paths.length}
          onClick={() => void refresh()}
        >
          {busy === "refresh" && <Loader2 className="h-4 w-4 animate-spin" />}
          {busy === "refresh" ? "Checking…" : "Check for changes"}
        </Button>
      </section>

      <section className="border-t border-white/[0.08] pt-7">
        <Link
          className="group flex items-center gap-4 rounded-xl border border-white/[0.08] bg-[linear-gradient(135deg,rgba(217,139,95,0.10),rgba(255,255,255,0.02))] px-4 py-4 transition-colors hover:border-white/[0.16]"
          href="/library/tools"
        >
          <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-mujin-copper/15 text-mujin-copper">
            <Wrench className="h-5 w-5" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block text-sm font-semibold text-white">
              Library tools
            </span>
            <span className="block text-xs leading-5 text-zinc-400">
              Skipped files, duplicates, album versions, the Albumizer, rescans
              and audio analysis.
            </span>
          </span>
          <ChevronRight className="h-4 w-4 text-zinc-500 transition-colors group-hover:text-white" />
        </Link>
      </section>

      <Dialog open={addOpen} onOpenChange={setAddOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Add a music folder</DialogTitle>
            <DialogDescription>
              Browse folders available on this server, then choose the folder
              that contains your music.
            </DialogDescription>
          </DialogHeader>
          <LibraryFolderBrowser
            busy={busy === "add"}
            initialDirectory="/"
            onCancel={() => setAddOpen(false)}
            onSelect={(selected) => void addLibrary(selected)}
          />
        </DialogContent>
      </Dialog>

      <Dialog
        open={Boolean(locatePath)}
        onOpenChange={(open) => {
          if (!open && busy !== "locate") setLocateRoot(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Where is this music now?</DialogTitle>
            <DialogDescription>
              It used to be in {locatePath}. Choose the folder that holds it on
              this server. Mujin Music finds each track at the same place inside
              the folder and moves playlists, favourites, history and edits over
              to it.
            </DialogDescription>
          </DialogHeader>
          <LibraryFolderBrowser
            busy={busy === "locate"}
            fallbackDirectories={
              locateRoot ? locateStartFolders(locateRoot).slice(1) : []
            }
            initialDirectory={
              locateRoot ? (locateStartFolders(locateRoot)[0] ?? "/") : "/"
            }
            key={locatePath ?? "none"}
            onCancel={() => setLocateRoot(null)}
            onSelect={(selected) =>
              locatePath ? void relocate(locatePath, selected) : undefined
            }
          />
        </DialogContent>
      </Dialog>

      <Dialog
        open={Boolean(removePath)}
        onOpenChange={(open) => {
          if (!open && busy !== "remove") setRemovePath(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Remove this music folder?</DialogTitle>
            <DialogDescription>
              Mujin Music will remove its indexed tracks from the library. Files
              on disk will not be changed.
            </DialogDescription>
          </DialogHeader>
          <p
            className="truncate rounded-md bg-white/[0.04] px-3 py-2 font-mono text-xs text-zinc-300"
            title={removePath ?? undefined}
          >
            {removePath}
          </p>
          <DialogFooter className="gap-2">
            <Button
              disabled={busy === "remove"}
              onClick={() => setRemovePath(null)}
              variant="outline"
            >
              Cancel
            </Button>
            <Button
              className="bg-red-600 text-white hover:bg-red-500"
              disabled={busy === "remove"}
              onClick={() => void removeFolder()}
            >
              {busy === "remove" ? "Removing…" : "Remove folder"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
