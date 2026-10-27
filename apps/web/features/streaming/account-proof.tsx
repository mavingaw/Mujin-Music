"use client";

import {
  accountProof,
  type AccountProofTone,
  type StreamingProviderStatus,
} from "@parson/music-sdk";
import { CircleDashed, ShieldAlert, ShieldCheck, ShieldX } from "lucide-react";

const tones: Record<
  AccountProofTone,
  { icon: typeof ShieldCheck; chip: string; text: string }
> = {
  verified: {
    icon: ShieldCheck,
    chip: "bg-emerald-400/10 text-emerald-300 ring-emerald-400/25",
    text: "text-emerald-200/80",
  },
  partial: {
    icon: ShieldAlert,
    chip: "bg-amber-400/10 text-amber-200 ring-amber-300/25",
    text: "text-amber-100/70",
  },
  failed: {
    icon: ShieldX,
    chip: "bg-rose-400/10 text-rose-200 ring-rose-300/25",
    text: "text-rose-100/70",
  },
  unproven: {
    icon: CircleDashed,
    chip: "bg-white/4 text-zinc-300 ring-white/12",
    text: "text-zinc-500",
  },
};

/**
 * Whether this account itself has been proven to stream whole tracks
 * losslessly, from its last connection check. Never claims more than the
 * check showed.
 */
export function AccountProofChip({
  provider,
  showDetail = false,
}: {
  provider: Pick<StreamingProviderStatus, "name" | "status" | "last_check">;
  showDetail?: boolean;
}) {
  const proof = accountProof(provider);
  if (!proof) return null;
  const tone = tones[proof.tone];
  const Icon = tone.icon;
  return (
    <div className="min-w-0">
      <span
        className={`inline-flex max-w-full items-center gap-1.5 rounded-full px-2.5 py-0.5 text-[0.7188rem] font-semibold tracking-wide ring-1 ring-inset ${tone.chip}`}
        data-proof={proof.tone}
        title={proof.detail}
      >
        <Icon aria-hidden="true" className="h-3.5 w-3.5 shrink-0" />
        <span className="truncate">{proof.label}</span>
      </span>
      {showDetail && (
        <p className={`mt-1.5 text-xs leading-5 ${tone.text}`}>
          {proof.detail}
        </p>
      )}
    </div>
  );
}
