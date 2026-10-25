"use client";

import { usePlayer } from "@/features/player/player-context";
import { RadioSwitch } from "@/features/player/radio/radio-controls";
import Link from "next/link";

function SettingRow({
  checked,
  description,
  label,
  onChange,
}: {
  checked: boolean;
  description: string;
  label: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className="flex items-start justify-between gap-6 py-4">
      <div>
        <p className="text-sm font-medium text-zinc-200">{label}</p>
        <p className="mt-1 max-w-md text-xs leading-5 text-zinc-500">
          {description}
        </p>
      </div>
      <RadioSwitch checked={checked} label={label} onChange={onChange} />
    </div>
  );
}

export default function RadioSettings() {
  const { radio } = usePlayer();
  return (
    <section aria-labelledby="radio-settings-title" className="mt-10 max-w-xl">
      <h2
        className="text-base font-semibold text-white"
        id="radio-settings-title"
      >
        Radio
      </h2>
      <div className="mt-1 divide-y divide-white/8">
        <SettingRow
          checked={radio.enabled}
          description="When your queue ends, keep playing similar music from your library. Radio pauses while repeat is on."
          label="Continue with Radio"
          onChange={radio.setEnabled}
        />
        <SettingRow
          checked={radio.comingUpNotice}
          description="Show the next Radio pick shortly before it plays, with thumbs up and down."
          label="Coming up notice"
          onChange={radio.setComingUpNotice}
        />
      </div>
      <div className="flex items-start justify-between gap-6 border-t border-white/8 py-4">
        <div>
          <p className="text-sm font-medium text-zinc-200">
            Bans and thumbs down
          </p>
          <p className="mt-1 max-w-md text-xs leading-5 text-zinc-500">
            Banned artists, albums and tracks never play in Radio, Shuffle or
            recommendations. Review them, unban, or undo a thumbs down.
          </p>
        </div>
        <Link
          className="shrink-0 rounded-full border border-white/15 px-3 py-1.5 text-xs font-semibold text-zinc-100 hover:border-white/40 hover:bg-white/6"
          href="/library?view=banned"
        >
          Manage
        </Link>
      </div>
      <p className="mt-2 text-xs leading-5 text-zinc-500">
        Thumbs down skips a track and keeps it out of Radio. Clearing your
        listening history in Data resets what Radio has learned; bans stay.
      </p>
    </section>
  );
}
