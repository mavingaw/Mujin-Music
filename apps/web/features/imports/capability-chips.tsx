"use client";

import {
  PROVIDER_CAPABILITY_LABELS,
  type CapabilityGrant,
} from "@parson/music-sdk";
import { Check, X } from "lucide-react";
import { CAPABILITY_ORDER } from "./import-review-state";

/** What a service's terms let Parson do, shown verbatim per capability. */
export default function CapabilityChips({
  capabilities,
  compact = false,
}: {
  capabilities: CapabilityGrant[];
  compact?: boolean;
}) {
  const ordered = [...capabilities].sort(
    (left, right) =>
      CAPABILITY_ORDER.indexOf(left.capability) -
      CAPABILITY_ORDER.indexOf(right.capability),
  );
  return (
    <ul
      aria-label="What Mujin Music can do with this service"
      className="flex flex-wrap gap-1.5"
    >
      {ordered.map((grant) => (
        <li
          className={`inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[0.6875rem] font-medium ${
            grant.available
              ? "border-emerald-400/25 bg-emerald-400/8 text-emerald-200"
              : "border-white/8 bg-white/2 text-zinc-500"
          }`}
          key={grant.capability}
          title={grant.detail}
        >
          {grant.available ? (
            <Check aria-hidden className="h-3 w-3" />
          ) : (
            <X aria-hidden className="h-3 w-3" />
          )}
          <span className={grant.available ? "" : "line-through"}>
            {PROVIDER_CAPABILITY_LABELS[grant.capability]}
          </span>
          <span className="sr-only">
            {grant.available ? " available: " : " not available: "}
            {grant.detail}
          </span>
          {!compact && grant.capability === "playback" && !grant.available && (
            <span className="text-zinc-600 no-underline"> · never</span>
          )}
        </li>
      ))}
    </ul>
  );
}
