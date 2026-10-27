"use client";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  getSpotifyDevices,
  handOffToSpotify,
  importErrorMessage,
  MAX_SPOTIFY_HANDOFF_TRACKS,
} from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { Loader2, MonitorSpeaker, RefreshCw } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { useImportServices } from "./service-connections";

/** True when Spotify is connected with the playback-control scope. */
export function useSpotifyHandoffAvailable() {
  const services = useImportServices();
  const spotify = services.data?.find((service) => service.id === "spotify");
  return Boolean(spotify?.connection?.details.can_handoff);
}

/**
 * Spotify Connect handoff: asks one of the person's own Spotify devices to
 * play the same tracks. Spotify plays them; Parson streams nothing.
 */
export default function SpotifyHandoffDialog({
  open,
  onOpenChange,
  songIds,
  positionMs,
  onHandedOff,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  songIds: string[];
  positionMs: number;
  onHandedOff?: () => void;
}) {
  const [selected, setSelected] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const devices = useQuery({
    queryKey: ["spotify-devices"],
    queryFn: getSpotifyDevices,
    enabled: open,
    retry: false,
  });
  const usable = (devices.data ?? []).filter(
    (device) => device.id && !device.is_restricted,
  );
  const active =
    selected ?? usable.find((device) => device.is_active)?.id ?? usable[0]?.id;
  const count = Math.min(songIds.length, MAX_SPOTIFY_HANDOFF_TRACKS);

  async function handOff() {
    setBusy(true);
    try {
      const result = await handOffToSpotify(songIds, active, positionMs);
      if (!result.started) {
        toast.error("None of these tracks were found on Spotify.");
        return;
      }
      toast.success(
        result.unresolved_song_ids.length
          ? `Playing ${result.resolved} tracks on Spotify. ${result.unresolved_song_ids.length} were not on Spotify.`
          : `Playing ${result.resolved} tracks on Spotify.`,
      );
      onHandedOff?.();
      onOpenChange(false);
    } catch (error) {
      toast.error(importErrorMessage(error, "Spotify did not start playing."));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Play on a Spotify device</DialogTitle>
          <DialogDescription>
            Spotify Connect will play the next {count} track
            {count === 1 ? "" : "s"} from Spotify&apos;s catalogue on your own
            device. Needs Spotify Premium. Mujin Music pauses here and streams
            nothing to Spotify.
          </DialogDescription>
        </DialogHeader>
        {devices.isPending ? (
          <div className="flex items-center gap-2 text-sm text-zinc-500">
            <Loader2 className="h-4 w-4 animate-spin" /> Looking for devices…
          </div>
        ) : devices.isError ? (
          <p className="text-sm text-zinc-400">
            {importErrorMessage(devices.error, "Devices could not be loaded.")}
          </p>
        ) : usable.length === 0 ? (
          <p className="text-sm text-zinc-400">
            No Spotify devices are online. Open Spotify on a phone, computer or
            speaker, then refresh.
          </p>
        ) : (
          <ul
            aria-label="Spotify devices"
            className="grid gap-1"
            role="radiogroup"
          >
            {usable.map((device) => (
              <li key={device.id}>
                <button
                  aria-checked={device.id === active}
                  className={`flex w-full items-center gap-3 rounded-md px-3 py-2 text-left text-sm ${
                    device.id === active
                      ? "bg-white/10 text-white"
                      : "text-zinc-300 hover:bg-white/5"
                  }`}
                  onClick={() => setSelected(device.id)}
                  role="radio"
                  type="button"
                >
                  <MonitorSpeaker className="h-4 w-4 text-zinc-500" />
                  <span className="min-w-0 flex-1 truncate">{device.name}</span>
                  <span className="text-xs text-zinc-500">
                    {device.is_active ? "Active" : device.type}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
        <DialogFooter className="gap-2">
          <Button
            disabled={devices.isFetching}
            onClick={() => void devices.refetch()}
            variant="ghost"
          >
            <RefreshCw /> Refresh
          </Button>
          <Button
            disabled={busy || !active || count === 0}
            onClick={() => void handOff()}
          >
            {busy && <Loader2 className="animate-spin" />}
            Play on Spotify
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
