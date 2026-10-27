"use client";

import { cn } from "@/lib/cn";
import type { TaggedEntity } from "@parson/music-sdk";
import { Tag } from "lucide-react";
import { useState } from "react";
import ManageTagsDialog from "./manage-tags-dialog";

/** Round "Tags" button for album and artist headers. */
export default function TagButton({
  className,
  entityIds,
  entityType,
  title,
}: {
  className?: string;
  entityIds: string[];
  entityType: TaggedEntity;
  title?: string;
}) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        aria-label="Manage tags"
        className={cn(
          "grid h-10 w-10 place-items-center rounded-full border border-white/15 text-zinc-300 transition-colors hover:border-white/35 hover:text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/40",
          className,
        )}
        onClick={() => setOpen(true)}
        title="Tags"
        type="button"
      >
        <Tag className="h-[18px] w-[18px]" />
      </button>
      {open && (
        <ManageTagsDialog
          entityIds={entityIds}
          entityType={entityType}
          onOpenChange={setOpen}
          open={open}
          title={title}
        />
      )}
    </>
  );
}
