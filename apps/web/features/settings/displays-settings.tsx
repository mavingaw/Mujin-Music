"use client";

import {
  displayLinkForZone,
  openZoneDisplay,
} from "@/features/display/display-links";
import { RadioSwitch } from "@/features/player/radio/radio-controls";
import { ZoneIcon } from "@/features/player/zones/zone-icons";
import { useZones } from "@/features/player/zones/zones-provider";
import getBaseURL from "@/lib/api/server-url";
import {
  approveDisplay,
  getDisplays,
  orderZones,
  removeDisplay,
  updateDisplay,
  type DisplayInfo,
  type Zone,
} from "@parson/music-sdk";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Copy, ExternalLink, Monitor, Trash2, Tv } from "lucide-react";
import { useState, type FormEvent } from "react";
import { toast } from "sonner";

const FOLLOW = "__follow__";

function errorMessage(error: unknown, fallback: string): string {
  const message = (error as { response?: { data?: { message?: string } } })
    ?.response?.data?.message;
  return message || fallback;
}

function ZoneSelect({
  id,
  label,
  onChange,
  value,
  zones,
}: {
  id: string;
  label: string;
  onChange: (zoneId: string | null) => void;
  value: string | null;
  zones: Zone[];
}) {
  const known = !value || zones.some((zone) => zone.id === value);
  return (
    <select
      aria-label={label}
      className="h-9 w-full min-w-0 rounded-lg border border-white/12 bg-mujin-surface-1 px-3 text-[0.8125rem] text-mujin-text outline-none transition-colors hover:border-white/25 focus-visible:border-mujin-copper/70"
      id={id}
      onChange={(event) =>
        onChange(event.target.value === FOLLOW ? null : event.target.value)
      }
      value={value ?? FOLLOW}
    >
      <option value={FOLLOW}>Follow what’s playing</option>
      {zones.map((zone) => (
        <option key={zone.id} value={zone.id}>
          {zone.name}
        </option>
      ))}
      {!known && <option value={value!}>A zone that is switched off</option>}
    </select>
  );
}

function lastSeen(display: DisplayInfo): string {
  if (display.online) return "On screen now";
  if (!display.last_seen_at) return "Not opened yet";
  const seen = new Date(`${display.last_seen_at.replace(" ", "T")}Z`);
  if (Number.isNaN(seen.getTime())) return "Offline";
  return `Last on ${seen.toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  })}`;
}

/**
 * Settings → Displays: pair a TV or tablet by the code it shows, open a
 * display in a new window or copy its link, and manage the screens that
 * follow your zones.
 */
export default function DisplaysSettings() {
  const queryClient = useQueryClient();
  const { zones: allZones, localZoneId } = useZones();
  const zones = orderZones(
    allZones.filter((zone) => !zone.grouped_into),
    localZoneId,
  );
  const displays = useQuery({
    queryKey: ["displays"],
    queryFn: getDisplays,
    refetchInterval: 15_000,
  });
  const refresh = () =>
    void queryClient.invalidateQueries({ queryKey: ["displays"] });
  const [code, setCode] = useState("");
  const [pairZone, setPairZone] = useState<string | null>(null);
  const [openZone, setOpenZone] = useState<string | null>(null);
  const approve = useMutation({
    mutationFn: () => approveDisplay({ code, zone_id: pairZone }),
    onSuccess: (display) => {
      setCode("");
      toast.success(`${display.name} is paired`, {
        description: "It shows what plays within a couple of seconds.",
      });
      refresh();
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Could not pair that screen.")),
  });
  const update = useMutation({
    mutationFn: ({
      id,
      patch,
    }: {
      id: string;
      patch: Parameters<typeof updateDisplay>[1];
    }) => updateDisplay(id, patch),
    onSuccess: refresh,
    onError: (error) =>
      toast.error(errorMessage(error, "Could not change that display.")),
  });
  const remove = useMutation({
    mutationFn: removeDisplay,
    onSuccess: refresh,
    onError: (error) =>
      toast.error(errorMessage(error, "Could not remove that display.")),
  });
  const zoneFor = (zoneId: string | null) =>
    zoneId ? (zones.find((zone) => zone.id === zoneId) ?? null) : null;
  const pair = (event: FormEvent) => {
    event.preventDefault();
    if (code.replace(/\D/g, "").length === 6) approve.mutate();
  };
  const host = getBaseURL().replace(/^https?:\/\//, "");

  return (
    <section aria-labelledby="displays-title" className="max-w-2xl">
      <h2 className="text-base font-semibold text-white" id="displays-title">
        Displays
      </h2>
      <p className="mt-1 max-w-xl text-[0.8125rem] leading-6 text-zinc-400">
        Turn a TV, tablet or spare screen into a now-playing display. It follows
        a zone and shows the artist photo, the artwork, the track and its
        format, a waveform and timed lyrics in large type. It has no controls
        and can see nothing else of your library.
      </p>

      <div className="mt-6 grid gap-4 md:grid-cols-2">
        <form
          aria-labelledby="pair-display-title"
          className="rounded-xl border border-white/[0.08] bg-mujin-raised p-4"
          onSubmit={pair}
        >
          <div className="flex items-center gap-2">
            <Tv aria-hidden="true" className="h-4 w-4 text-mujin-copper" />
            <h3
              className="text-sm font-medium text-mujin-text"
              id="pair-display-title"
            >
              Pair a screen
            </h3>
          </div>
          <p className="mt-2 text-xs leading-5 text-zinc-500">
            On the TV or tablet, open{" "}
            <span className="font-mono text-zinc-300">{host}/display</span> in
            its browser and enter the six digits it shows.
          </p>
          <label className="sr-only" htmlFor="display-code">
            Pairing code
          </label>
          <input
            autoComplete="off"
            className="mt-3 h-11 w-full rounded-lg border border-white/12 bg-mujin-surface-1 px-3 text-center font-mono text-xl tracking-[0.4em] text-mujin-text outline-none placeholder:text-zinc-600 focus-visible:border-mujin-copper/70"
            id="display-code"
            inputMode="numeric"
            maxLength={7}
            onChange={(event) =>
              setCode(event.target.value.replace(/[^\d ]/g, "").slice(0, 7))
            }
            placeholder="000000"
            value={code}
          />
          <div className="mt-3">
            <ZoneSelect
              id="pair-display-zone"
              label="Zone the paired screen shows"
              onChange={setPairZone}
              value={pairZone}
              zones={zones}
            />
          </div>
          <button
            className="mj-primary mt-3 h-9 w-full rounded-full text-[0.8125rem] font-semibold disabled:opacity-50"
            disabled={code.replace(/\D/g, "").length !== 6 || approve.isPending}
            type="submit"
          >
            {approve.isPending ? "Pairing…" : "Pair"}
          </button>
        </form>

        <div className="rounded-xl border border-white/[0.08] bg-mujin-raised p-4">
          <div className="flex items-center gap-2">
            <Monitor aria-hidden="true" className="h-4 w-4 text-mujin-copper" />
            <h3 className="text-sm font-medium text-mujin-text">
              Open a display link
            </h3>
          </div>
          <p className="mt-2 text-xs leading-5 text-zinc-500">
            Open it in a window here, or copy the link to any browser on your
            network. The link holds the display’s key: share it like a password.
          </p>
          <div className="mt-3">
            <ZoneSelect
              id="open-display-zone"
              label="Zone the display link shows"
              onChange={setOpenZone}
              value={openZone}
              zones={zones}
            />
          </div>
          <div className="mt-3 grid grid-cols-2 gap-2">
            <button
              className="flex h-9 items-center justify-center gap-2 rounded-full border border-white/15 text-[0.8125rem] font-medium text-mujin-text hover:border-white/35 hover:bg-white/[0.05]"
              onClick={() => {
                const zone = zoneFor(openZone);
                void openZoneDisplay(zone)
                  .then(refresh)
                  .catch((error) =>
                    toast.error(
                      errorMessage(error, "Could not open a display."),
                    ),
                  );
              }}
              type="button"
            >
              <ExternalLink aria-hidden="true" className="h-3.5 w-3.5" /> Open
            </button>
            <button
              className="flex h-9 items-center justify-center gap-2 rounded-full border border-white/15 text-[0.8125rem] font-medium text-mujin-text hover:border-white/35 hover:bg-white/[0.05]"
              onClick={() => {
                void displayLinkForZone(zoneFor(openZone))
                  .then(async (link) => {
                    await navigator.clipboard.writeText(link);
                    toast.success("Display link copied");
                    refresh();
                  })
                  .catch((error) =>
                    toast.error(
                      errorMessage(error, "Could not copy the link."),
                    ),
                  );
              }}
              type="button"
            >
              <Copy aria-hidden="true" className="h-3.5 w-3.5" /> Copy link
            </button>
          </div>
        </div>
      </div>

      <h3 className="mt-8 text-sm font-medium text-mujin-text">
        Your displays
      </h3>
      {displays.isPending ? (
        <p className="mt-3 text-xs text-zinc-500">Loading…</p>
      ) : displays.isError ? (
        <p className="mt-3 text-xs text-zinc-500">
          Could not load your displays.
        </p>
      ) : displays.data.length === 0 ? (
        <p className="mt-3 text-xs leading-5 text-zinc-500">
          No displays yet. Pair a screen or open a display link above.
        </p>
      ) : (
        <ul className="mt-3 divide-y divide-white/[0.07] rounded-xl border border-white/[0.08] bg-mujin-raised">
          {displays.data.map((display) => {
            const zone = zoneFor(display.zone_id);
            return (
              <li
                className="grid gap-3 p-4 sm:grid-cols-[minmax(0,1fr)_260px]"
                key={display.id}
              >
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span
                      aria-hidden="true"
                      className={`h-2 w-2 shrink-0 rounded-full ${display.online ? "bg-quality-lossless shadow-[0_0_0_3px_rgb(217_139_95/0.2)]" : "bg-zinc-600"}`}
                    />
                    <input
                      aria-label="Display name"
                      className="min-w-0 flex-1 truncate rounded bg-transparent text-sm font-medium text-mujin-text outline-none focus-visible:bg-white/[0.05]"
                      defaultValue={display.name}
                      maxLength={60}
                      onBlur={(event) => {
                        const name = event.target.value.trim();
                        if (name && name !== display.name)
                          update.mutate({ id: display.id, patch: { name } });
                      }}
                    />
                  </div>
                  <p className="mt-1 flex items-center gap-1.5 text-xs text-zinc-500">
                    {zone && <ZoneIcon className="h-3.5 w-3.5" zone={zone} />}
                    {lastSeen(display)}
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  <ZoneSelect
                    id={`display-zone-${display.id}`}
                    label={`Zone ${display.name} shows`}
                    onChange={(zoneId) =>
                      update.mutate({
                        id: display.id,
                        patch: { zone_id: zoneId },
                      })
                    }
                    value={display.zone_id}
                    zones={zones}
                  />
                  <button
                    aria-label={`Remove ${display.name}`}
                    className="grid h-9 w-9 shrink-0 place-items-center rounded-full text-zinc-500 hover:bg-white/10 hover:text-mujin-text"
                    onClick={() => remove.mutate(display.id)}
                    title="Remove: the screen stops showing and must pair again"
                    type="button"
                  >
                    <Trash2 className="h-4 w-4" />
                  </button>
                </div>
                <div className="flex items-center justify-between gap-4 sm:col-span-2">
                  <p className="text-xs text-zinc-500">
                    Timed lyrics over the artwork when a track has them
                  </p>
                  <RadioSwitch
                    checked={display.lyrics === "auto"}
                    label={`Lyrics on ${display.name}`}
                    onChange={(checked) =>
                      update.mutate({
                        id: display.id,
                        patch: { lyrics: checked ? "auto" : "off" },
                      })
                    }
                  />
                </div>
              </li>
            );
          })}
        </ul>
      )}

      <div className="mt-6 rounded-xl border border-white/[0.06] p-4 text-xs leading-5 text-zinc-500">
        <p className="font-medium text-zinc-300">What has been tested</p>
        <p className="mt-1">
          Displays were tested in desktop Chromium at TV (1920×1080), tablet and
          phone sizes, following a browser zone through track changes, pauses
          and pairing. Smart-TV browsers and Chromecast have not been tested
          yet. On a Chromecast, open the link in Chrome on a computer and choose
          Cast → Cast tab; a dedicated Mujin Music receiver app does not exist
          yet.
        </p>
      </div>
    </section>
  );
}
