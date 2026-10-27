"use client";

import {
  checkLiveStream,
  liveFormatText,
  looksLikeStreamUrl,
  parseTagInput,
  type LiveStreamCheck,
} from "@parson/music-sdk";
import {
  CheckCircle2,
  Loader2,
  Music2,
  Radio,
  ShieldAlert,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useSession } from "@/features/account/session-provider";
import { stationHref } from "./live-radio-model";
import { QualityBadge } from "./station-ui";
import { liveRadioError, useAddLiveStation } from "./use-live-radio";

const fieldClass =
  "h-10 w-full rounded-lg border border-white/10 bg-white/[0.04] px-3 text-sm text-white outline-none transition-colors placeholder:text-zinc-600 focus:border-white/30";

function CheckResult({ check }: { check: LiveStreamCheck }) {
  return (
    <div className="rounded-xl border border-emerald-400/20 bg-emerald-500/[0.06] p-4">
      <p className="flex items-center gap-2 text-sm font-semibold text-emerald-200">
        <CheckCircle2 className="h-4 w-4" />
        {check.stream_name ?? "Stream found"}
      </p>
      <div className="mt-3 flex flex-wrap items-center gap-2 text-xs text-zinc-300">
        <QualityBadge quality={check.quality} />
        <span>{liveFormatText(check.format)}</span>
        {check.format.verified && (
          <span className="text-zinc-500">read from the audio</span>
        )}
        {check.transport === "hls" && (
          <span className="rounded border border-white/10 px-1.5 text-[10px] font-semibold uppercase">
            HLS
          </span>
        )}
      </div>
      {check.title && (
        <p className="mt-3 flex items-center gap-2 truncate text-sm text-white">
          <Music2 className="h-4 w-4 shrink-0 text-zinc-400" />
          <span className="truncate">Now playing: {check.title}</span>
        </p>
      )}
      {!check.titles_supported && (
        <p className="mt-2 text-xs text-zinc-500">
          This stream does not send track titles.
        </p>
      )}
      {check.resolved_url && (
        <p
          className="mt-2 truncate text-[11px] text-zinc-500"
          title={check.resolved_url}
        >
          Plays from {check.resolved_url}
        </p>
      )}
    </div>
  );
}

/**
 * Adds a station by URL. The server opens the URL first (following PLS,
 * M3U, ASX and HLS playlists and redirects, never into private networks
 * unless an administrator allows a LAN stream) and shows what it found.
 */
export function AddStationDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { session } = useSession();
  const isAdmin = session?.role === "admin";
  const router = useRouter();
  const add = useAddLiveStation();
  const [url, setUrl] = useState("");
  const [name, setName] = useState("");
  const [logo, setLogo] = useState("");
  const [homepage, setHomepage] = useState("");
  const [tags, setTags] = useState("");
  const [allowLan, setAllowLan] = useState(false);
  const [check, setCheck] = useState<LiveStreamCheck | null>(null);
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reset = () => {
    setUrl("");
    setName("");
    setLogo("");
    setHomepage("");
    setTags("");
    setAllowLan(false);
    setCheck(null);
    setError(null);
  };

  const runCheck = async () => {
    if (!looksLikeStreamUrl(url)) {
      setError("Enter a full stream URL starting with http:// or https://.");
      return;
    }
    setChecking(true);
    setError(null);
    setCheck(null);
    try {
      const result = await checkLiveStream(url.trim(), allowLan);
      setCheck(result);
      if (!name && result.stream_name) setName(result.stream_name);
      if (!tags && result.genre) setTags(result.genre.toLowerCase());
    } catch (failure) {
      setError(liveRadioError(failure, "Parson could not open that stream."));
    } finally {
      setChecking(false);
    }
  };

  const save = () => {
    setError(null);
    add.mutate(
      {
        url: url.trim(),
        name: name.trim() || undefined,
        logo_url: logo.trim() || undefined,
        homepage: homepage.trim() || undefined,
        tags: parseTagInput(tags),
        allow_lan: allowLan || undefined,
      },
      {
        onSuccess: ({ station }) => {
          onOpenChange(false);
          reset();
          router.push(stationHref(station.id));
        },
        onError: (failure) =>
          setError(liveRadioError(failure, "The station could not be added.")),
      },
    );
  };

  return (
    <Dialog
      onOpenChange={(next) => {
        onOpenChange(next);
        if (!next) reset();
      }}
      open={open}
    >
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-xl">
            <Radio className="h-5 w-5 text-rose-300" /> Add a station
          </DialogTitle>
          <DialogDescription className="text-sm leading-6 text-zinc-400">
            Paste a stream URL: MP3, AAC, Ogg, Opus or FLAC streams, PLS, M3U
            and ASX playlists, or HLS (.m3u8). Parson checks it and shows what
            it really sends.
          </DialogDescription>
        </DialogHeader>
        <form
          className="space-y-4"
          onSubmit={(event) => {
            event.preventDefault();
            if (check) save();
            else void runCheck();
          }}
        >
          <label className="block space-y-1.5">
            <span className="text-xs font-semibold uppercase tracking-wide text-zinc-400">
              Stream URL
            </span>
            <div className="flex gap-2">
              <input
                autoFocus
                className={fieldClass}
                inputMode="url"
                onChange={(event) => {
                  setUrl(event.target.value);
                  setCheck(null);
                }}
                placeholder="https://stream.example.com/live.pls"
                value={url}
              />
              <Button
                disabled={!url.trim() || checking}
                onClick={() => void runCheck()}
                variant="secondary"
              >
                {checking ? <Loader2 className="animate-spin" /> : "Check"}
              </Button>
            </div>
          </label>
          {isAdmin && (
            <label className="flex items-start gap-2 text-sm text-zinc-300">
              <input
                checked={allowLan}
                className="mt-1 accent-white"
                onChange={(event) => {
                  setAllowLan(event.target.checked);
                  setCheck(null);
                }}
                type="checkbox"
              />
              <span>
                This stream is on my local network
                <span className="block text-xs text-zinc-500">
                  Lets Parson reach private addresses (a NAS or Icecast at home)
                  for this station only.
                </span>
              </span>
            </label>
          )}
          {error && (
            <p className="flex items-start gap-2 rounded-lg border border-rose-400/20 bg-rose-500/[0.06] p-3 text-sm text-rose-200">
              <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" />
              {error}
            </p>
          )}
          {check && <CheckResult check={check} />}
          {check && (
            <div className="grid gap-3 sm:grid-cols-2">
              <label className="block space-y-1.5 sm:col-span-2">
                <span className="text-xs font-semibold uppercase tracking-wide text-zinc-400">
                  Name
                </span>
                <input
                  className={fieldClass}
                  onChange={(event) => setName(event.target.value)}
                  placeholder="Station name"
                  value={name}
                />
              </label>
              <label className="block space-y-1.5">
                <span className="text-xs font-semibold uppercase tracking-wide text-zinc-400">
                  Logo URL
                </span>
                <input
                  className={fieldClass}
                  onChange={(event) => setLogo(event.target.value)}
                  placeholder="https://…/logo.png"
                  value={logo}
                />
              </label>
              <label className="block space-y-1.5">
                <span className="text-xs font-semibold uppercase tracking-wide text-zinc-400">
                  Website
                </span>
                <input
                  className={fieldClass}
                  onChange={(event) => setHomepage(event.target.value)}
                  placeholder="https://…"
                  value={homepage}
                />
              </label>
              <label className="block space-y-1.5 sm:col-span-2">
                <span className="text-xs font-semibold uppercase tracking-wide text-zinc-400">
                  Genres
                </span>
                <input
                  className={fieldClass}
                  onChange={(event) => setTags(event.target.value)}
                  placeholder="jazz, late night"
                  value={tags}
                />
              </label>
            </div>
          )}
          <div className="flex justify-end gap-2 pt-1">
            <Button onClick={() => onOpenChange(false)} variant="ghost">
              Cancel
            </Button>
            <Button disabled={!check || add.isPending} type="submit">
              {add.isPending && <Loader2 className="animate-spin" />}
              Add to My Live Radio
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
