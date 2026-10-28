"use client";

import {
  createProfile,
  deleteProfile,
  updateProfile,
  validProfilePin,
  type Profile,
  type ProfileColor,
  type UpdateProfileInput,
} from "@parson/music-sdk";
import { useQueryClient } from "@tanstack/react-query";
import {
  ArrowLeftRight,
  Check,
  KeyRound,
  Loader2,
  Lock,
  Pencil,
  Plus,
  Trash2,
} from "lucide-react";
import { useSearchParams } from "next/navigation";
import { useEffect, useState, type FormEvent } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { RadioSwitch } from "@/features/player/radio/radio-controls";
import { cn } from "@/lib/cn";
import { ProfileAvatar } from "./profile-avatar";
import { profileBadge, profileSubtitle } from "./profiles-model";
import { UnlockEntry } from "./pin-entry";
import { profilesKey, useProfileSwitch, useProfiles } from "./use-profiles";

function errorMessage(error: unknown, fallback: string): string {
  const data = (error as { response?: { data?: { message?: unknown } } })
    ?.response?.data;
  return typeof data?.message === "string" ? data.message : fallback;
}

function ColorPicker({
  colors,
  name,
  onChange,
  value,
}: {
  colors: ProfileColor[];
  name: string;
  onChange: (color: ProfileColor) => void;
  value: ProfileColor;
}) {
  return (
    <div
      aria-label="Profile colour"
      className="flex flex-wrap gap-2"
      role="radiogroup"
    >
      {colors.map((color) => (
        <button
          aria-checked={value === color}
          aria-label={color}
          className={cn(
            "rounded-full p-0.5 ring-2 transition-all",
            value === color
              ? "ring-white"
              : "ring-transparent hover:ring-white/30",
          )}
          key={color}
          onClick={() => onChange(color)}
          role="radio"
          type="button"
        >
          <ProfileAvatar color={color} name={name || "?"} size={28} />
        </button>
      ))}
    </div>
  );
}

function ProfileDialog({
  colors,
  onClose,
  open,
  profile,
}: {
  colors: ProfileColor[];
  onClose: () => void;
  open: boolean;
  /** Editing this profile; adding a new one when absent. */
  profile?: Profile;
}) {
  const queryClient = useQueryClient();
  const [name, setName] = useState(profile?.name ?? "");
  const [color, setColor] = useState<ProfileColor>(
    profile?.color ??
      colors[Math.floor(Math.random() * colors.length)] ??
      "violet",
  );
  const [pin, setPin] = useState("");
  const [pinMode, setPinMode] = useState<"keep" | "set" | "remove">(
    profile ? "keep" : "set",
  );
  const [quickSwitch, setQuickSwitch] = useState(profile?.quickSwitch ?? true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const pinValid = pinMode !== "set" || pin === "" || validProfilePin(pin);
  const valid = name.trim().length > 0 && name.trim().length <= 40 && pinValid;

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!valid || busy) return;
    setBusy(true);
    setError(null);
    try {
      if (profile) {
        const input: UpdateProfileInput = { name: name.trim(), color };
        if (pinMode === "set" && pin) input.pin = pin;
        if (pinMode === "remove") input.pin = null;
        if (profile.kind === "account") input.quickSwitch = quickSwitch;
        await updateProfile(profile.id, input);
        toast.success(`${name.trim()} updated`);
      } else {
        await createProfile({
          name: name.trim(),
          color,
          pin: pin || undefined,
        });
        toast.success(`${name.trim()} added`, {
          description: "Switch to it from the profile button at the top right.",
        });
      }
      await queryClient.invalidateQueries({ queryKey: profilesKey });
      onClose();
    } catch (cause) {
      setError(errorMessage(cause, "Could not save the profile."));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog onOpenChange={(next) => !next && onClose()} open={open}>
      <DialogContent className="max-w-md">
        <DialogTitle>{profile ? "Edit profile" : "Add a profile"}</DialogTitle>
        <DialogDescription>
          {profile
            ? "Name, colour and how switching to this profile is protected."
            : "For someone who shares this server. They get their own history, favourites, Radio and scrobbling, without a password of their own."}
        </DialogDescription>
        <form className="grid gap-5" onSubmit={submit}>
          <div className="flex items-center gap-4">
            <ProfileAvatar color={color} name={name || "?"} size={56} />
            <div className="min-w-0 flex-1">
              <label
                className="text-xs font-medium text-zinc-400"
                htmlFor="profile-name"
              >
                Name
              </label>
              <Input
                autoFocus
                className="mt-1"
                id="profile-name"
                maxLength={40}
                onChange={(event) => setName(event.target.value)}
                placeholder="Mia"
                value={name}
              />
            </div>
          </div>
          <div>
            <p className="mb-2 text-xs font-medium text-zinc-400">Colour</p>
            <ColorPicker
              colors={colors}
              name={name}
              onChange={setColor}
              value={color}
            />
          </div>
          <div>
            <p className="text-xs font-medium text-zinc-400">PIN</p>
            {profile?.hasPin && pinMode === "keep" ? (
              <div className="mt-2 flex items-center gap-2">
                <span className="flex items-center gap-1.5 rounded-full border border-white/10 px-2.5 py-1 text-xs text-zinc-300">
                  <Lock className="h-3 w-3" /> PIN set
                </span>
                <Button
                  onClick={() => setPinMode("set")}
                  size="sm"
                  variant="ghost"
                >
                  Change
                </Button>
                <Button
                  onClick={() => setPinMode("remove")}
                  size="sm"
                  variant="ghost"
                >
                  Remove
                </Button>
              </div>
            ) : pinMode === "remove" ? (
              <p className="mt-2 text-xs text-zinc-400">
                The PIN will be removed.{" "}
                <button
                  className="text-zinc-200 underline-offset-2 hover:underline"
                  onClick={() => setPinMode("keep")}
                  type="button"
                >
                  Keep it
                </button>
              </p>
            ) : (
              <>
                <Input
                  autoComplete="new-password"
                  // PIN spacing for entered digits only, so the
                  // placeholder reads as a word.
                  className={cn("mt-1", pin && "tracking-[0.4em]")}
                  inputMode="numeric"
                  maxLength={8}
                  onChange={(event) =>
                    setPin(event.target.value.replace(/\D/g, "").slice(0, 8))
                  }
                  onFocus={() => setPinMode("set")}
                  placeholder={profile?.hasPin ? "New PIN" : "Optional"}
                  type="password"
                  value={pin}
                />
                <p
                  className={cn(
                    "mt-1.5 text-[0.6875rem]",
                    pinValid ? "text-zinc-500" : "text-rose-300",
                  )}
                >
                  4 to 8 digits. With a PIN, switching to this profile asks for
                  it first.
                </p>
              </>
            )}
          </div>
          {profile?.kind === "account" && (
            <div className="flex items-start justify-between gap-4">
              <div>
                <p className="text-sm font-medium text-zinc-200">
                  Show in the profile switcher
                </p>
                <p className="mt-1 text-xs leading-5 text-zinc-500">
                  Hidden accounts are only reached by signing in or with their
                  password.
                </p>
              </div>
              <RadioSwitch
                checked={quickSwitch}
                label="Show in the profile switcher"
                onChange={setQuickSwitch}
              />
            </div>
          )}
          {error && (
            <p className="text-sm text-rose-300" role="alert">
              {error}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <Button onClick={onClose} variant="ghost">
              Cancel
            </Button>
            <Button disabled={!valid || busy} type="submit">
              {busy && <Loader2 className="animate-spin" />}
              {profile ? "Save" : "Add profile"}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function ProfileCard({
  onEdit,
  onRemove,
  profile,
}: {
  onEdit: () => void;
  onRemove: () => void;
  profile: Profile;
}) {
  const switchTo = useProfileSwitch();
  const [unlocking, setUnlocking] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const badge = profileBadge(profile);

  const attempt = async (secret?: string) => {
    setBusy(true);
    setError(null);
    const result = await switchTo(
      profile,
      profile.unlock === "pin"
        ? { pin: secret }
        : profile.unlock === "password"
          ? { password: secret }
          : {},
    );
    setBusy(false);
    if (result.status) setUnlocking(false);
    else setError(result.message ?? "Could not switch profiles.");
  };

  return (
    <li
      className={cn(
        "group relative overflow-hidden rounded-2xl border p-4 transition-colors",
        profile.isCurrent
          ? "border-mujin-copper/30 bg-mujin-copper/[0.04]"
          : "border-white/8 bg-white/[0.02] hover:border-white/15",
      )}
    >
      <div className="flex items-center gap-3">
        <ProfileAvatar color={profile.color} name={profile.name} size={48} />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <p className="truncate text-sm font-semibold text-white">
              {profile.name}
            </p>
            {badge && (
              <span className="shrink-0 rounded-full border border-white/10 px-1.5 py-px text-[0.625rem] font-medium uppercase tracking-wide text-zinc-400 max-sm:hidden">
                {badge}
              </span>
            )}
          </div>
          <p className="mt-0.5 truncate text-xs text-zinc-500">
            {profileSubtitle(profile)}
          </p>
          {!profile.quickSwitch && (
            <p className="mt-0.5 text-[0.6875rem] text-zinc-600">
              Hidden from the switcher
            </p>
          )}
        </div>
        {profile.isCurrent ? (
          <span className="flex items-center gap-1 rounded-full bg-mujin-copper/15 px-2 py-1 text-[0.6875rem] font-semibold text-mujin-copper">
            <Check aria-label="This device" className="h-3 w-3" />
            <span className="max-sm:hidden">This device</span>
          </span>
        ) : (
          <Button
            disabled={busy}
            onClick={() =>
              profile.unlock === "none"
                ? void attempt()
                : setUnlocking((value) => !value)
            }
            size="sm"
            variant="secondary"
          >
            {busy && !unlocking ? (
              <Loader2 className="animate-spin" />
            ) : profile.unlock === "none" ? (
              <ArrowLeftRight />
            ) : (
              <KeyRound />
            )}
            Switch
          </Button>
        )}
      </div>
      {unlocking && (
        <UnlockEntry
          busy={busy}
          error={error}
          mode={profile.unlock === "pin" ? "pin" : "password"}
          onSubmit={(value) => void attempt(value)}
        />
      )}
      {error && !unlocking && (
        <p className="mt-2 text-xs text-rose-300" role="alert">
          {error}
        </p>
      )}
      {(profile.canEdit || profile.canDelete) && (
        <div className="mt-3 flex gap-1 border-t border-white/6 pt-2">
          {profile.canEdit && (
            <Button onClick={onEdit} size="sm" variant="ghost">
              <Pencil /> Edit
            </Button>
          )}
          {profile.canDelete && (
            <Button
              className="hover:text-rose-300"
              onClick={onRemove}
              size="sm"
              variant="ghost"
            >
              <Trash2 /> Remove
            </Button>
          )}
        </div>
      )}
    </li>
  );
}

export default function ProfilesSettings() {
  const profiles = useProfiles();
  const queryClient = useQueryClient();
  const requestedAdd = useSearchParams().get("add") === "1";
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<Profile | null>(null);
  const [removing, setRemoving] = useState<Profile | null>(null);
  const [removeBusy, setRemoveBusy] = useState(false);

  useEffect(() => {
    if (requestedAdd && profiles.data?.canCreate) setAdding(true);
  }, [requestedAdd, profiles.data?.canCreate]);

  const data = profiles.data;
  const remove = async () => {
    if (!removing) return;
    setRemoveBusy(true);
    try {
      await deleteProfile(removing.id);
      toast.success(`${removing.name} removed`);
      await queryClient.invalidateQueries({ queryKey: profilesKey });
      setRemoving(null);
    } catch (cause) {
      toast.error(errorMessage(cause, "Could not remove the profile."));
    } finally {
      setRemoveBusy(false);
    }
  };

  return (
    <section aria-labelledby="profiles-title" className="max-w-3xl">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h2
            className="text-base font-semibold text-white"
            id="profiles-title"
          >
            Profiles
          </h2>
          <p className="mt-1 max-w-xl text-sm leading-6 text-zinc-400">
            Everyone who listens here gets a profile with their own history,
            favourites, ratings, Radio taste, recommendations and scrobbling.
            Switching changes only this device, and the music keeps playing.
          </p>
        </div>
        {data?.canCreate && (
          <Button onClick={() => setAdding(true)} size="sm">
            <Plus /> Add profile
          </Button>
        )}
      </div>

      {profiles.isPending ? (
        <div className="mt-6 grid gap-3 sm:grid-cols-2">
          {[0, 1].map((index) => (
            <div
              className="h-24 animate-pulse rounded-2xl bg-white/[0.03]"
              key={index}
            />
          ))}
        </div>
      ) : profiles.isError || !data ? (
        <p className="mt-6 text-sm text-rose-300">
          Profiles could not be loaded.
        </p>
      ) : (
        <ul className="mt-6 grid gap-3 sm:grid-cols-2">
          {data.profiles.map((profile) => (
            <ProfileCard
              key={profile.id}
              onEdit={() => setEditing(profile)}
              onRemove={() => setRemoving(profile)}
              profile={profile}
            />
          ))}
        </ul>
      )}

      <div className="mt-6 grid gap-2 rounded-2xl border border-white/6 bg-white/[0.015] p-4 text-xs leading-5 text-zinc-500 sm:grid-cols-3">
        <p>
          <span className="font-medium text-zinc-300">Instant switching.</span>{" "}
          Profiles without a PIN open in one click from the button at the top
          right.
        </p>
        <p>
          <span className="font-medium text-zinc-300">PINs.</span> Add a PIN to
          any profile. Administrators without a PIN ask for their password.
        </p>
        <p>
          <span className="font-medium text-zinc-300">Scrobbling.</span> Each
          profile connects its own Last.fm and ListenBrainz under Scrobbling.
        </p>
      </div>

      {data && (adding || editing) && (
        <ProfileDialog
          colors={data.colors}
          key={editing?.id ?? "new"}
          onClose={() => {
            setAdding(false);
            setEditing(null);
          }}
          open
          profile={editing ?? undefined}
        />
      )}

      <Dialog
        onOpenChange={(next) => !next && setRemoving(null)}
        open={removing !== null}
      >
        <DialogContent className="max-w-sm">
          <DialogTitle>Remove {removing?.name}?</DialogTitle>
          <DialogDescription>
            Their listening history, favourites, playlists, Radio taste and
            scrobbling connections are deleted. A safety backup is made first.
          </DialogDescription>
          <div className="flex justify-end gap-2">
            <Button onClick={() => setRemoving(null)} variant="ghost">
              Cancel
            </Button>
            <Button
              disabled={removeBusy}
              onClick={() => void remove()}
              variant="destructive"
            >
              {removeBusy && <Loader2 className="animate-spin" />}
              Remove
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </section>
  );
}
