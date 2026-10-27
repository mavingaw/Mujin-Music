"use client";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/cn";
import type { FolderBackup, FolderBackupListing } from "@parson/music-sdk";
import { Archive, FolderSearch, Loader2, Upload } from "lucide-react";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
} from "react";
import { describeBackups, formatBytes } from "./migration-presentation";

export type PickedBackup =
  { kind: "folder"; backup: FolderBackup } | { kind: "file"; file: File };

/**
 * Finds a Parson backup to restore: in the folders the server knows about,
 * in any folder the server can see (a mounted NAS share), or as a file
 * uploaded from this device.
 */
export default function BackupPicker({
  list,
  selected,
  onSelect,
}: {
  list: (path: string) => Promise<FolderBackupListing>;
  selected: PickedBackup | null;
  onSelect: (backup: PickedBackup | null) => void;
}) {
  const fileInput = useRef<HTMLInputElement>(null);
  const [path, setPath] = useState("");
  const [listing, setListing] = useState<FolderBackupListing | null>(null);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const request = useRef(0);

  const load = useCallback(
    async (folder: string) => {
      const id = ++request.current;
      setLoading(true);
      setFailed(false);
      try {
        const next = await list(folder);
        if (id === request.current) setListing(next);
      } catch {
        if (id === request.current) {
          setListing(null);
          setFailed(true);
        }
      } finally {
        if (id === request.current) setLoading(false);
      }
    },
    [list],
  );

  useEffect(() => {
    void load("");
  }, [load]);

  function search(event: FormEvent) {
    event.preventDefault();
    void load(path.trim());
  }

  const backups = listing?.backups ?? [];
  const descriptions = describeBackups(backups);
  const where = listing?.path
    ? listing.path
    : listing?.suggested_folders.length
      ? listing.suggested_folders.join(", ")
      : "";

  return (
    <div className="grid gap-4">
      <form className="flex gap-2" onSubmit={search}>
        <Input
          aria-label="Folder to look in"
          autoComplete="off"
          className="font-mono text-[13px]"
          onChange={(event) => setPath(event.target.value)}
          placeholder="Folder on the server, e.g. /mnt/nas/parson-backups"
          spellCheck={false}
          value={path}
        />
        <Button disabled={loading} type="submit" variant="secondary">
          <FolderSearch />
          Look
        </Button>
      </form>

      <div className="overflow-hidden rounded-xl border border-white/10">
        {loading ? (
          <div className="flex h-24 items-center justify-center gap-2 text-sm text-zinc-500">
            <Loader2 className="h-4 w-4 animate-spin" />
            Looking for backups…
          </div>
        ) : backups.length ? (
          <ul
            aria-label="Backups found"
            className="max-h-72 divide-y divide-white/[0.06] overflow-y-auto"
            role="listbox"
          >
            {backups.map((backup, index) => {
              const active =
                selected?.kind === "folder" &&
                selected.backup.path === backup.path;
              return (
                <li key={backup.path}>
                  <button
                    aria-selected={active}
                    className={cn(
                      "flex w-full items-start gap-3 px-4 py-3 text-left transition-colors",
                      active ? "bg-white/[0.08]" : "hover:bg-white/[0.04]",
                    )}
                    onClick={() =>
                      onSelect(active ? null : { kind: "folder", backup })
                    }
                    role="option"
                    type="button"
                  >
                    <span
                      className={cn(
                        "mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg",
                        active
                          ? "bg-white text-black"
                          : "bg-white/[0.06] text-zinc-300",
                      )}
                    >
                      <Archive className="h-4 w-4" />
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-medium text-white">
                        {descriptions[index]?.title}
                      </span>
                      <span className="mt-0.5 block truncate text-xs text-zinc-500">
                        {descriptions[index]?.detail}
                      </span>
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        ) : (
          <p className="px-4 py-5 text-sm leading-6 text-zinc-500">
            {failed
              ? "The server could not look there."
              : where
                ? `No Mujin Music backups in ${where}.`
                : "No backups found yet."}{" "}
            Type the folder that holds them (a NAS share must be mounted on the
            server), or upload a backup file.
          </p>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-3">
        <Button
          onClick={() => fileInput.current?.click()}
          size="sm"
          variant="ghost"
        >
          <Upload />
          Upload a backup file
        </Button>
        {selected?.kind === "file" ? (
          <span className="truncate text-sm text-zinc-300">
            {selected.file.name} · {formatBytes(selected.file.size)}
          </span>
        ) : null}
        <input
          accept=".zst,.tar.zst,application/zstd"
          className="hidden"
          onChange={(event) => {
            const file = event.target.files?.[0];
            if (file) onSelect({ kind: "file", file });
            event.currentTarget.value = "";
          }}
          ref={fileInput}
          type="file"
        />
      </div>
    </div>
  );
}
