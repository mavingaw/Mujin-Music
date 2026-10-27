"use client";

import { switcherProfiles, type Profile } from "@parson/music-sdk";
import {
  Check,
  Loader2,
  Lock,
  Plus,
  Settings2,
  UsersRound,
} from "lucide-react";
import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/cn";
import { ProfileAvatar } from "./profile-avatar";
import { profileBadge, profileSubtitle } from "./profiles-model";
import { UnlockEntry } from "./pin-entry";
import { useProfileSwitch, useProfiles } from "./use-profiles";

/**
 * The profile button at the top right, like Roon's: shows who is
 * listening on this device and switches to anyone else in one click (or a
 * PIN). Only this device changes profile; the music keeps playing.
 */
export function ProfileSwitcher() {
  const [open, setOpen] = useState(false);
  const profiles = useProfiles();
  const panel = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const current = profiles.data?.profiles.find((profile) => profile.isCurrent);

  useEffect(() => {
    if (!open) return;
    void profiles.refetch();
    const close = (event: PointerEvent) => {
      const target = event.target as Node;
      if (
        !panel.current?.contains(target) &&
        !button.current?.contains(target)
      ) {
        setOpen(false);
      }
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(false);
        button.current?.focus();
      }
    };
    document.addEventListener("pointerdown", close);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", close);
      document.removeEventListener("keydown", escape);
    };
    // Refetch once per opening.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  if (!current) return null;

  return (
    <div className="electron-titlebar-no-drag relative">
      <button
        aria-expanded={open}
        aria-haspopup="dialog"
        aria-label={`Profile: ${current.name}. Switch profile`}
        className={cn(
          "flex h-9 items-center gap-2 rounded-full p-0.5 transition-all duration-150 hover:bg-white/8 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-white/30 md:pr-3",
          open && "bg-white/8",
        )}
        onClick={() => setOpen((value) => !value)}
        ref={button}
        title="Switch profile"
        type="button"
      >
        <ProfileAvatar color={current.color} name={current.name} size={32} />
        <span className="hidden max-w-28 truncate text-sm font-medium text-zinc-200 md:block">
          {current.name}
        </span>
      </button>
      {open && profiles.data && (
        <div
          aria-label="Switch profile"
          className="absolute right-0 top-[calc(100%+8px)] z-60 w-[min(340px,calc(100vw-24px))] origin-top-right animate-[profile-panel-in_160ms_ease-out] overflow-hidden rounded-2xl border border-white/10 bg-[#0c0c0e]/95 shadow-[0_24px_60px_-12px_rgba(0,0,0,0.85)] backdrop-blur-xl"
          ref={panel}
          role="dialog"
        >
          <SwitcherPanel
            canCreate={profiles.data.canCreate}
            onDone={() => setOpen(false)}
            profiles={switcherProfiles(profiles.data)}
          />
        </div>
      )}
    </div>
  );
}

function SwitcherPanel({
  canCreate,
  onDone,
  profiles,
}: {
  canCreate: boolean;
  onDone: () => void;
  profiles: Profile[];
}) {
  const switchTo = useProfileSwitch();
  const [unlocking, setUnlocking] = useState<number | null>(null);
  const [busy, setBusy] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const attempt = async (profile: Profile, secret?: string) => {
    setBusy(profile.id);
    setError(null);
    const result = await switchTo(
      profile,
      profile.unlock === "pin"
        ? { pin: secret }
        : profile.unlock === "password"
          ? { password: secret }
          : {},
    );
    setBusy(null);
    if (result.status) {
      setUnlocking(null);
      onDone();
    } else {
      setError(result.message ?? "Could not switch profiles.");
    }
  };

  const choose = (profile: Profile) => {
    if (profile.isCurrent || busy !== null) return;
    setError(null);
    if (profile.unlock === "none") {
      void attempt(profile);
    } else {
      setUnlocking((value) => (value === profile.id ? null : profile.id));
    }
  };

  return (
    <div>
      <div className="flex items-center justify-between px-4 pb-2 pt-3.5">
        <p className="text-[11px] font-semibold uppercase tracking-[0.14em] text-zinc-500">
          Who&apos;s listening?
        </p>
        <UsersRound aria-hidden="true" className="h-3.5 w-3.5 text-zinc-600" />
      </div>
      <ul className="max-h-[min(60vh,420px)] overflow-y-auto px-1.5 pb-1.5">
        {profiles.map((profile) => {
          const badge = profileBadge(profile);
          const expanded = unlocking === profile.id;
          return (
            <li key={profile.id}>
              <div
                className={cn(
                  "rounded-xl transition-colors",
                  expanded ? "bg-white/6" : "",
                )}
              >
                <button
                  aria-current={profile.isCurrent ? "true" : undefined}
                  className={cn(
                    "group flex w-full items-center gap-3 rounded-xl px-2.5 py-2 text-left transition-colors",
                    profile.isCurrent
                      ? "cursor-default"
                      : "hover:bg-white/6 focus-visible:bg-white/6 focus-visible:outline-none",
                  )}
                  disabled={busy !== null && busy !== profile.id}
                  onClick={() => choose(profile)}
                  type="button"
                >
                  <span className="relative">
                    <ProfileAvatar
                      className={cn(
                        "transition-transform duration-200",
                        !profile.isCurrent && "group-hover:scale-105",
                      )}
                      color={profile.color}
                      name={profile.name}
                      size={40}
                    />
                    {profile.isCurrent && (
                      <span className="absolute -bottom-0.5 -right-0.5 flex h-4 w-4 items-center justify-center rounded-full bg-sky-400 ring-2 ring-[#0c0c0e]">
                        <Check
                          className="h-2.5 w-2.5 text-black"
                          strokeWidth={3.5}
                        />
                      </span>
                    )}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="flex items-center gap-2">
                      <span className="truncate text-sm font-semibold text-white">
                        {profile.name}
                      </span>
                      {badge && (
                        <span className="shrink-0 rounded-full border border-white/10 px-1.5 py-px text-[10px] font-medium uppercase tracking-wide text-zinc-400">
                          {badge}
                        </span>
                      )}
                    </span>
                    <span className="mt-0.5 block truncate text-xs text-zinc-500">
                      {profile.isCurrent
                        ? `Listening now · ${profileSubtitle(profile)}`
                        : profileSubtitle(profile)}
                    </span>
                  </span>
                  {busy === profile.id && !expanded ? (
                    <Loader2 className="h-4 w-4 animate-spin text-zinc-400" />
                  ) : profile.unlock !== "none" && !profile.isCurrent ? (
                    <Lock
                      aria-label={
                        profile.unlock === "pin"
                          ? "PIN required"
                          : "Password required"
                      }
                      className="h-3.5 w-3.5 text-zinc-500"
                    />
                  ) : null}
                </button>
                {expanded && (
                  <div className="px-3 pb-2">
                    <UnlockEntry
                      busy={busy === profile.id}
                      error={error}
                      mode={profile.unlock === "pin" ? "pin" : "password"}
                      onSubmit={(value) => void attempt(profile, value)}
                    />
                  </div>
                )}
              </div>
            </li>
          );
        })}
      </ul>
      {error && unlocking === null && (
        <p className="px-4 pb-2 text-xs text-rose-300" role="alert">
          {error}
        </p>
      )}
      <div className="flex items-center gap-1 border-t border-white/8 p-1.5">
        {canCreate && (
          <Link
            className="flex flex-1 items-center gap-2 rounded-lg px-2.5 py-2 text-xs font-medium text-zinc-300 transition-colors hover:bg-white/6 hover:text-white"
            href="/settings?tab=profiles&add=1"
            onClick={onDone}
          >
            <Plus className="h-3.5 w-3.5" />
            Add profile
          </Link>
        )}
        <Link
          className="flex flex-1 items-center gap-2 rounded-lg px-2.5 py-2 text-xs font-medium text-zinc-300 transition-colors hover:bg-white/6 hover:text-white"
          href="/settings?tab=profiles"
          onClick={onDone}
        >
          <Settings2 className="h-3.5 w-3.5" />
          Manage profiles
        </Link>
      </div>
    </div>
  );
}
