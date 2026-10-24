"use client";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { AboutCredit, AudioProperties } from "@parson/music-sdk/types";
import { CreditsList } from "./credits-list";
import { formatAudio } from "./credit-roles";

/** Per-track credits, the "View credits…" sheet from Roon's track menu. */
export function TrackCreditsDialog({
  open,
  onOpenChange,
  title,
  artistName,
  credits,
  isrc,
  audio,
  label,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  artistName: string;
  credits: readonly AboutCredit[];
  isrc?: string | null;
  audio?: AudioProperties | null;
  label?: string | null;
}) {
  const format = formatAudio(audio);
  const facts = [
    format && { label: "Format", value: format },
    label && { label: "Label", value: label },
    isrc && { label: "ISRC", value: isrc },
  ].filter(Boolean) as { label: string; value: string }[];
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{artistName}</DialogDescription>
        </DialogHeader>
        <CreditsList
          credits={credits}
          emptyMessage="This track has no credits in its tags yet. Turn on online metadata in Settings → Library to add MusicBrainz credits."
          showTracks={false}
        />
        {facts.length > 0 && (
          <dl className="mt-2 grid grid-cols-[auto_minmax(0,1fr)] gap-x-6 gap-y-1 border-t border-white/8 pt-4 text-sm">
            {facts.map((fact) => (
              <div className="contents" key={fact.label}>
                <dt className="text-zinc-500">{fact.label}</dt>
                <dd className="truncate text-zinc-200">{fact.value}</dd>
              </div>
            ))}
          </dl>
        )}
      </DialogContent>
    </Dialog>
  );
}
