"use client";

import {
  anyZonePlaying,
  deviceVolumeLabel,
  dspSummary,
  isDspActive,
  sleepTimerLabel,
  groupingPlan,
  streamQuality,
  orderZones,
  zoneKindLabel,
  zoneQualityLabel,
  zoneSliderMax,
  zoneStatusLabel,
  type Zone,
} from "@parson/music-sdk";
import {
  ArrowRightLeft,
  AudioLines,
  Check,
  Cpu,
  Link2,
  Lock,
  Minus,
  Moon,
  MoreHorizontal,
  Pause,
  Plus,
  RefreshCw,
  Settings2,
  SlidersHorizontal,
  Unlink,
  Volume2,
  VolumeX,
} from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { toast } from "sonner";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Slider } from "@/components/ui/slider";
import { ZoneIcon } from "./zone-icons";
import { ZoneOutputsPanel } from "./zone-outputs";
import { ZoneSettingsDialog } from "./zone-settings";
import { QualityLight, ZoneSignalPathDialog } from "./zone-signal-path";
import { useZones } from "./zones-provider";

const SLEEP_CHOICES = [15, 30, 45, 60, 90] as const;

function errorMessage(error: unknown, fallback: string): string {
  const response = (
    error as { response?: { data?: { error?: string; message?: string } } }
  )?.response?.data;
  const code = response?.message || response?.error;
  if (!code) return fallback;
  const known: Record<string, string> = {
    zone_has_no_parson_queue: "That zone has nothing queued to transfer.",
    zone_cannot_receive_transfer:
      "Chromecast playback starts from the Parson mobile app.",
    chromecast_groups_are_made_in_google_home:
      "Group Chromecast speakers in the Google Home app.",
    bridges_disabled:
      "Parson Bridge support is turned off on this server (PARSON_BRIDGES=off).",
    end_of_queue: "That was the last track in the queue.",
    sonos_disabled: "Sonos support is turned off on this server.",
    private_zones_cannot_be_grouped:
      "Private zones stay out of groups. Turn off Private zone in its settings first.",
    fixed_volume:
      "This zone has fixed volume. Set the level on the amplifier, or choose another volume mode in Device setup.",
    library_not_ready: "The library is still loading. Try again shortly.",
    zone_device_error: "The device did not respond.",
  };
  return known[code] ?? code.replaceAll("_", " ");
}

/** The footer zone button (Roon's zone picker). */
export function ZonePickerButton({ compact = false }: { compact?: boolean }) {
  const { zones, remoteZone, localZoneId } = useZones();
  const [open, setOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const local = zones.find((zone) => zone.id === localZoneId);
  const current = remoteZone ?? local;
  const label = remoteZone ? remoteZone.name : "This device";
  return (
    <>
      <button
        aria-label={`Zone: ${label}. Choose where music plays`}
        className={`group/control relative flex h-8 min-w-0 items-center gap-1.5 rounded-full px-2 text-xs font-medium transition-colors ${remoteZone ? "bg-emerald-400/15 text-emerald-300 hover:bg-emerald-400/25" : "text-zinc-400 hover:bg-white/10 hover:text-white"}`}
        data-zone-picker
        onClick={() => setOpen(true)}
        onContextMenu={(event) => {
          // Right-click opens the zone's settings, as in Roon.
          if (!current) return;
          event.preventDefault();
          setSettingsOpen(true);
        }}
        title="Choose a zone (right-click for zone settings)"
        type="button"
      >
        <ZoneIcon zone={current ?? { kind: "web" }} />
        {!compact && (
          <span className="hidden max-w-28 truncate lg:inline">{label}</span>
        )}
        {remoteZone?.group && (
          <span
            aria-hidden
            className={`h-1.5 w-1.5 rounded-full ${remoteZone.group.synced ? "bg-emerald-400" : "bg-amber-400"}`}
          />
        )}
      </button>
      <ZonePickerDialog open={open} onOpenChange={setOpen} />
      <ZoneSettingsDialog
        onOpenChange={setSettingsOpen}
        open={settingsOpen}
        zone={current ?? null}
      />
    </>
  );
}

export function ZonePickerDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const zones = useZones();
  const [grouping, setGrouping] = useState(false);
  const [checked, setChecked] = useState<string[]>([]);
  const [editing, setEditing] = useState<string | null>(null);
  const [editFocus, setEditFocus] = useState<"dsp" | "device" | undefined>();
  const [pausing, setPausing] = useState(false);
  const ordered = useMemo(
    () => orderZones(zones.zones, zones.localZoneId),
    [zones.localZoneId, zones.zones],
  );
  const active = zones.selectedZoneId;
  const activeZone = ordered.find((zone) => zone.id === active) ?? null;
  const transferable =
    activeZone?.source === "parson" && activeZone.queue.song_ids.length > 0;
  const plan = groupingPlan(
    checked
      .map((id) => ordered.find((zone) => zone.id === id))
      .filter((zone): zone is Zone => Boolean(zone)),
  );

  useEffect(() => {
    if (!open) {
      setGrouping(false);
      setChecked([]);
      setEditing(null);
    }
  }, [open]);

  const run = async (task: () => Promise<unknown>, failure: string) => {
    try {
      await task();
    } catch (error) {
      toast.error(errorMessage(error, failure));
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-xl gap-3 p-0 sm:p-0">
        <DialogHeader className="border-b border-white/10 px-5 pb-3 pt-5">
          <DialogTitle>Zones</DialogTitle>
          <DialogDescription>
            {grouping
              ? "Pick the zones that should play together."
              : "Choose where music plays, move playback between rooms, and control any zone from here."}
          </DialogDescription>
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <button
              className={`flex h-8 items-center gap-1.5 rounded-full border px-3 text-xs ${grouping ? "border-white bg-white text-black" : "border-white/15 text-zinc-300 hover:bg-white/10"}`}
              onClick={() => {
                setGrouping((value) => !value);
                setChecked(active ? [active] : []);
              }}
              type="button"
            >
              <Link2 className="h-3.5 w-3.5" />
              {grouping ? "Cancel grouping" : "Group zones"}
            </button>
            <button
              aria-label="Pause all zones"
              className="flex h-8 items-center gap-1.5 rounded-full border border-white/15 px-3 text-xs text-zinc-300 hover:bg-white/10 disabled:opacity-40"
              disabled={pausing || !anyZonePlaying(ordered)}
              onClick={() =>
                void run(async () => {
                  setPausing(true);
                  try {
                    const paused = await zones.pauseAll();
                    toast.success(
                      paused === 1 ? "Paused 1 zone" : `Paused ${paused} zones`,
                    );
                  } finally {
                    setPausing(false);
                  }
                }, "Could not pause every zone.")
              }
              type="button"
            >
              <Pause className="h-3.5 w-3.5" />
              Pause all
            </button>
            {zones.outputs?.upnp.enabled && (
              <button
                className="flex h-8 items-center gap-1.5 rounded-full border border-white/15 px-3 text-xs text-zinc-300 hover:bg-white/10 disabled:opacity-50"
                disabled={zones.busy}
                onClick={() =>
                  void run(zones.rescanOutputs, "Looking for renderers failed.")
                }
                type="button"
              >
                <AudioLines className="h-3.5 w-3.5" />
                Find renderers
              </button>
            )}
            {zones.sonos?.enabled && (
              <button
                className="flex h-8 items-center gap-1.5 rounded-full border border-white/15 px-3 text-xs text-zinc-300 hover:bg-white/10 disabled:opacity-50"
                disabled={zones.busy}
                onClick={() =>
                  void run(zones.rescanSonos, "Sonos discovery failed.")
                }
                type="button"
              >
                <RefreshCw
                  className={`h-3.5 w-3.5 ${zones.busy ? "animate-spin" : ""}`}
                />
                Find Sonos
              </button>
            )}
          </div>
        </DialogHeader>
        <ul className="max-h-[min(60dvh,560px)] space-y-1.5 overflow-y-auto px-3 pb-2">
          {zones.loading && !ordered.length && (
            <li className="px-3 py-6 text-center text-sm text-zinc-500">
              Looking for zones…
            </li>
          )}
          {ordered
            .filter((zone) => !grouping || zone.kind !== "linked")
            .map((zone) => (
              <ZoneRow
                active={zone.id === active}
                checked={checked.includes(zone.id)}
                editing={editing === zone.id}
                grouping={grouping}
                isLocal={zone.id === zones.localZoneId}
                key={zone.id}
                onCheck={(value) =>
                  setChecked((current) =>
                    value
                      ? [...current, zone.id]
                      : current.filter((id) => id !== zone.id),
                  )
                }
                editFocus={editFocus}
                onEdit={(value, focus) => {
                  setEditFocus(focus);
                  setEditing(value ? zone.id : null);
                }}
                onSelect={() => {
                  zones.selectZone(zone.id);
                  onOpenChange(false);
                }}
                onTransfer={
                  transferable &&
                  active &&
                  zone.id !== active &&
                  zone.capabilities.transfer_target
                    ? () =>
                        void run(async () => {
                          await zones.transfer(active, zone.id);
                          toast.success(`Moved playback to ${zone.name}`);
                        }, "Transfer failed.")
                    : undefined
                }
                run={run}
                zone={zone}
              />
            ))}
        </ul>
        {grouping ? (
          <div className="border-t border-white/10 px-5 py-4">
            <p
              className={`text-xs leading-5 ${plan.allowed ? (plan.synced ? "text-emerald-300" : "text-amber-300") : "text-zinc-500"}`}
              role="status"
            >
              {plan.reason}
            </p>
            <div className="mt-3 flex justify-end">
              <button
                className="h-9 rounded-full bg-white px-4 text-sm font-semibold text-black disabled:opacity-40"
                disabled={!plan.allowed || zones.busy}
                onClick={() =>
                  void run(async () => {
                    const groupId = await zones.group(checked);
                    zones.selectZone(groupId);
                    setGrouping(false);
                    setChecked([]);
                    toast.success(
                      plan.synced
                        ? "Grouped. Rooms play sample-synced."
                        : "Linked. Zones follow the same controls.",
                    );
                  }, "Grouping failed.")
                }
                type="button"
              >
                Save group
              </button>
            </div>
          </div>
        ) : (
          <ZoneFooterNotes />
        )}
      </DialogContent>
    </Dialog>
  );
}

function ZoneFooterNotes() {
  const { sonos, outputs } = useZones();
  if (!sonos && !outputs) return null;
  const devices = outputs?.devices;
  const upnp = outputs?.upnp;
  return (
    <div className="max-h-[32dvh] space-y-1 overflow-y-auto border-t border-white/10 px-5 py-3 text-[11px] leading-4 text-zinc-500">
      <ZoneOutputsPanel />
      {sonos && (
        <p>
          {sonos.enabled
            ? sonos.players
              ? `${sonos.players} Sonos ${sonos.players === 1 ? "room" : "rooms"} on this network. Stereo pairs count as one room.`
              : "No Sonos found. Sonos must be on the same network as the Parson server, and the server must accept connections from it (UDP 1900 for discovery and the server port for audio)."
            : "Sonos support is turned off on this server (PARSON_SONOS=off)."}
        </p>
      )}
      {upnp?.enabled && (
        <p>
          {upnp.renderers
            ? `${upnp.renderers} UPnP/DLNA ${upnp.renderers === 1 ? "renderer" : "renderers"} found.`
            : "No UPnP/DLNA renderers found yet. They need to be on the server's network (UDP 1900)."}
        </p>
      )}
      {devices && (
        <p>
          {devices.enabled && devices.backend
            ? `${devices.devices} audio ${devices.devices === 1 ? "device" : "devices"} on the server's computer (${devices.backend}). Each one is its own zone.`
            : (devices.note ??
              "Audio devices of this computer show as zones in the Parson desktop app.")}
        </p>
      )}
    </div>
  );
}

function ZoneRow({
  active,
  checked,
  editFocus,
  editing,
  grouping,
  isLocal,
  onCheck,
  onEdit,
  onSelect,
  onTransfer,
  run,
  zone,
}: {
  active: boolean;
  checked: boolean;
  editFocus?: "dsp" | "device";
  editing: boolean;
  grouping: boolean;
  isLocal: boolean;
  onCheck: (value: boolean) => void;
  onEdit: (value: boolean, focus?: "dsp" | "device") => void;
  onSelect: () => void;
  onTransfer?: () => void;
  run: (task: () => Promise<unknown>, failure: string) => Promise<void>;
  zone: Zone;
}) {
  const zones = useZones();
  const nowPlaying = zone.now_playing;
  const subtitle = nowPlaying
    ? [
        zone.status === "playing" ? "" : zoneStatusLabel(zone),
        nowPlaying.title,
        nowPlaying.artist,
      ]
        .filter(Boolean)
        .join(" · ")
    : zoneStatusLabel(zone);
  const device =
    zone.group && zone.kind === "sonos"
      ? `Sonos · ${zone.group.members.length} rooms`
      : zone.device || zoneKindLabel(zone.kind);
  const groupable = zone.capabilities.group && zone.kind !== "linked";
  const quality = zoneQualityLabel(zone);
  const dspOn = isDspActive(zone.settings?.dsp);
  const deviceVolume = deviceVolumeLabel(zone.volume);
  return (
    <li
      className={`rounded-xl border px-3 py-2.5 ${active ? "border-white/25 bg-white/[0.06]" : "border-white/[0.06] hover:border-white/15"} ${zone.grouped_into ? "ml-6" : ""}`}
      data-zone-id={zone.id}
    >
      <div className="flex items-center gap-3">
        {grouping ? (
          <Checkbox
            aria-label={`Include ${zone.name}`}
            checked={checked}
            disabled={!groupable}
            onCheckedChange={(value) => onCheck(value === true)}
          />
        ) : null}
        <button
          className="flex min-w-0 flex-1 items-center gap-3 text-left"
          data-zone-select={zone.id}
          disabled={grouping}
          onClick={onSelect}
          type="button"
        >
          <span
            className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-lg ${zone.status === "playing" ? "bg-emerald-400/15 text-emerald-300" : "bg-white/[0.06] text-zinc-400"}`}
          >
            <ZoneIcon zone={zone} />
          </span>
          <span className="min-w-0 flex-1">
            <span className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
              <span className="max-w-full truncate text-sm font-medium text-white">
                {zone.name}
              </span>
              {isLocal && <Badge>This device</Badge>}
              {zone.settings?.private && (
                <Badge>
                  <Lock className="mr-0.5 inline h-2.5 w-2.5" />
                  Private
                </Badge>
              )}
              {zone.group &&
                (zone.group.synced ? (
                  <Badge tone="synced">Synced group</Badge>
                ) : (
                  <Badge tone="linked">Linked · not sample-synced</Badge>
                ))}
              {quality && (
                <Badge
                  tone={
                    quality === "Bit-perfect" || quality === "Lossless"
                      ? "synced"
                      : "linked"
                  }
                >
                  {quality}
                </Badge>
              )}
              {dspOn && <Badge>DSP</Badge>}
              {active && <Check className="h-3.5 w-3.5 text-white" />}
            </span>
            <span className="block truncate text-xs text-zinc-400">
              {subtitle}
            </span>
            <span className="block truncate text-[11px] text-zinc-600">
              {device}
              {zone.sleep_timer
                ? ` · ${sleepTimerLabel(zone.sleep_timer, Date.now())}`
                : ""}
            </span>
          </span>
        </button>
        {!grouping && onTransfer && (
          <button
            aria-label={`Transfer playback to ${zone.name}`}
            className="flex h-8 items-center gap-1 rounded-full border border-white/15 px-2.5 text-xs text-zinc-300 hover:bg-white/10"
            onClick={onTransfer}
            title="Move the queue, track and position here"
            type="button"
          >
            <ArrowRightLeft className="h-3.5 w-3.5" />
            <span className="hidden sm:inline">Transfer here</span>
          </button>
        )}
        {!grouping && (
          <ZoneMenu
            onEdit={(focus) => onEdit(!editing, focus)}
            run={run}
            zone={zone}
          />
        )}
      </div>
      {!grouping && zone.volume && zone.capabilities.volume && (
        <ZoneVolumeRow
          label={zone.group ? "Group volume" : "Volume"}
          muted={zone.volume.muted}
          max={zoneSliderMax(zone.volume)}
          onStep={(delta) =>
            void run(
              () => zones.send(zone.id, { command: "adjust_volume", delta }),
              "Could not change volume.",
            )
          }
          step={zone.volume.step}
          onMute={(muted) =>
            void run(
              () => zones.send(zone.id, { command: "set_mute", muted }),
              "Could not change mute.",
            )
          }
          onVolume={(volume) =>
            void run(
              () => zones.send(zone.id, { command: "set_volume", volume }),
              "Could not change volume.",
            )
          }
          volume={zone.volume.level}
        />
      )}
      {!grouping && deviceVolume && (
        <p className="mt-1 pl-12 text-[11px] tabular-nums text-zinc-600">
          {deviceVolume}
        </p>
      )}
      {!grouping && zone.group && zone.group.members.length > 1 && (
        <div className="mt-2 space-y-1 border-l border-white/10 pl-3">
          {zone.group.members.map((member) => (
            <div className="flex items-center gap-2" key={member.id}>
              <span className="w-24 truncate text-[11px] text-zinc-400">
                {member.name}
              </span>
              {member.volume && (
                <div className="flex-1">
                  <ZoneVolumeRow
                    dense
                    label={`${member.name} volume`}
                    max={zoneSliderMax(member.volume)}
                    muted={member.volume.muted}
                    onStep={(delta) =>
                      void run(
                        () =>
                          zones.send(zone.id, {
                            command: "adjust_volume",
                            delta,
                            member_id: member.id,
                          }),
                        "Could not change volume.",
                      )
                    }
                    step={member.volume.step}
                    onMute={(muted) =>
                      void run(
                        () =>
                          zones.send(zone.id, {
                            command: "set_mute",
                            muted,
                            member_id: member.id,
                          }),
                        "Could not change mute.",
                      )
                    }
                    onVolume={(volume) =>
                      void run(
                        () =>
                          zones.send(zone.id, {
                            command: "set_volume",
                            volume,
                            member_id: member.id,
                          }),
                        "Could not change volume.",
                      )
                    }
                    volume={member.volume.level}
                  />
                </div>
              )}
              <button
                aria-label={`Remove ${member.name} from the group`}
                className="flex h-7 w-7 items-center justify-center rounded-full text-zinc-500 hover:bg-white/10 hover:text-white"
                onClick={() =>
                  void run(
                    () => zones.ungroup(zone.group!.id, member.id),
                    "Could not change the group.",
                  )
                }
                type="button"
              >
                <Unlink className="h-3.5 w-3.5" />
              </button>
            </div>
          ))}
          <p className="pt-1 text-[11px] leading-4 text-zinc-500">
            {zone.group.sync_note}
          </p>
        </div>
      )}
      {!grouping && active && zone.output?.stream && (
        <SignalPathSummary zone={zone} />
      )}
      {!grouping && active && zone.notes.length > 0 && (
        <ul className="mt-2 space-y-1">
          {zone.notes
            .filter((note) => note !== zone.group?.sync_note)
            .map((note) => (
              <li className="text-[11px] leading-4 text-zinc-500" key={note}>
                {note}
              </li>
            ))}
        </ul>
      )}
      <ZoneSettingsDialog
        focus={editFocus}
        onOpenChange={(value) => onEdit(value)}
        open={editing}
        zone={zone}
      />
    </li>
  );
}

/** One line of the playing track's path; opens the full signal path. */
function SignalPathSummary({ zone }: { zone: Zone }) {
  const [open, setOpen] = useState(false);
  const stream = zone.output?.stream;
  if (!stream) return null;
  return (
    <>
      <button
        className="mt-2 flex w-full items-start gap-2 rounded-lg px-1 py-1 text-left text-[11px] leading-4 text-zinc-400 transition hover:bg-white/[0.04] hover:text-zinc-200"
        data-signal-path-summary
        onClick={() => setOpen(true)}
        type="button"
      >
        <QualityLight className="mt-1" quality={streamQuality(stream)} />
        <span className="min-w-0 flex-1">
          {[stream.source, ...stream.steps, stream.output].join(" → ")}
        </span>
        <span className="shrink-0 font-medium text-zinc-300 underline-offset-2 hover:underline">
          Signal path
        </span>
      </button>
      <ZoneSignalPathDialog onOpenChange={setOpen} open={open} zone={zone} />
    </>
  );
}

function Badge({
  children,
  tone = "neutral",
}: {
  children: ReactNode;
  tone?: "neutral" | "synced" | "linked";
}) {
  const tones = {
    neutral: "border-white/15 text-zinc-400",
    synced: "border-emerald-400/30 text-emerald-300",
    linked: "border-amber-400/30 text-amber-300",
  } as const;
  return (
    <span
      className={`shrink-0 rounded-full border px-1.5 py-px text-[10px] font-medium ${tones[tone]}`}
    >
      {children}
    </span>
  );
}

function ZoneVolumeRow({
  dense = false,
  label,
  max,
  muted,
  onMute,
  onStep,
  onVolume,
  step,
  volume,
}: {
  dense?: boolean;
  label: string;
  /** Where the slider stops: the comfort limit, else the safety maximum. */
  max: number;
  muted: boolean;
  onMute: (muted: boolean) => void;
  /** +/- buttons: may go past the comfort limit, never the safety maximum. */
  onStep: (delta: number) => void;
  onVolume: (volume: number) => void;
  step: number;
  volume: number;
}) {
  const [draft, setDraft] = useState<number | null>(null);
  useEffect(() => setDraft(null), [volume]);
  const shown = draft ?? volume;
  const aboveComfort = shown > max;
  const stepButton =
    "flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-zinc-500 hover:bg-white/10 hover:text-white";
  return (
    <div className={`flex items-center gap-1.5 ${dense ? "" : "mt-2 pl-12"}`}>
      <button
        aria-label={muted ? `Unmute ${label}` : `Mute ${label}`}
        className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full text-zinc-400 hover:bg-white/10 hover:text-white"
        onClick={() => onMute(!muted)}
        type="button"
      >
        {muted ? (
          <VolumeX className="h-3.5 w-3.5" />
        ) : (
          <Volume2 className="h-3.5 w-3.5" />
        )}
      </button>
      <button
        aria-label={`Lower ${label}`}
        className={stepButton}
        onClick={() => onStep(-Math.max(1, step))}
        type="button"
      >
        <Minus className="h-3 w-3" />
      </button>
      <Slider
        aria-label={label}
        className="h-7 flex-1 **:data-[slot=slider-range]:bg-white **:data-[slot=slider-thumb]:h-3 **:data-[slot=slider-thumb]:w-3 **:data-[slot=slider-thumb]:border-0 **:data-[slot=slider-thumb]:bg-white **:data-[slot=slider-track]:h-1 **:data-[slot=slider-track]:bg-white/15"
        max={100}
        min={0}
        onValueChange={([value]) => setDraft(Math.min(value ?? 0, max))}
        onValueCommit={([value]) => onVolume(Math.min(value ?? 0, max))}
        value={[muted ? 0 : shown]}
      />
      <button
        aria-label={`Raise ${label}`}
        className={stepButton}
        onClick={() => onStep(Math.max(1, step))}
        type="button"
      >
        <Plus className="h-3 w-3" />
      </button>
      <span
        className={`w-14 text-right text-[11px] tabular-nums ${aboveComfort ? "text-amber-300" : "text-zinc-500"}`}
        title={
          max < 100
            ? `The slider stops at ${max}; + goes past it up to the safety limit.`
            : undefined
        }
      >
        {muted ? "Muted" : shown}
        {max < 100 && !muted ? `/${max}` : ""}
      </span>
    </div>
  );
}

function ZoneMenu({
  onEdit,
  run,
  zone,
}: {
  onEdit: (focus?: "dsp" | "device") => void;
  run: (task: () => Promise<unknown>, failure: string) => Promise<void>;
  zone: Zone;
}) {
  const zones = useZones();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          aria-label={`${zone.name} options`}
          className="flex h-8 w-8 items-center justify-center rounded-full text-zinc-400 hover:bg-white/10 hover:text-white"
          type="button"
        >
          <MoreHorizontal className="h-4 w-4" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="z-120 w-56">
        {zone.capabilities.sleep_timer && (
          <>
            <DropdownMenuLabel className="flex items-center gap-2">
              <Moon className="h-3.5 w-3.5" /> Sleep timer
            </DropdownMenuLabel>
            {SLEEP_CHOICES.map((minutes) => (
              <DropdownMenuItem
                key={minutes}
                onSelect={() =>
                  void run(
                    () => zones.setSleepTimer(zone.id, minutes),
                    "Could not set the sleep timer.",
                  )
                }
              >
                {minutes} minutes
              </DropdownMenuItem>
            ))}
            {zone.sleep_timer && (
              <DropdownMenuItem
                onSelect={() =>
                  void run(
                    () => zones.setSleepTimer(zone.id, null),
                    "Could not cancel the sleep timer.",
                  )
                }
              >
                Cancel sleep timer
              </DropdownMenuItem>
            )}
            <DropdownMenuSeparator />
          </>
        )}
        <DropdownMenuItem onSelect={() => onEdit()}>
          <Settings2 className="h-4 w-4" /> Zone settings
        </DropdownMenuItem>
        {zone.settings?.supports.dsp && (
          <DropdownMenuItem onSelect={() => onEdit("dsp")}>
            <SlidersHorizontal className="h-4 w-4" /> DSP
            <span className="ml-auto max-w-24 truncate text-[11px] text-zinc-500">
              {dspSummary(zone.settings.dsp)}
            </span>
          </DropdownMenuItem>
        )}
        {(zone.settings?.supports.exclusive ||
          zone.settings?.supports.max_format) && (
          <DropdownMenuItem onSelect={() => onEdit("device")}>
            <Cpu className="h-4 w-4" /> Device setup
          </DropdownMenuItem>
        )}
        {zone.group && (
          <DropdownMenuItem
            onSelect={() =>
              void run(
                () => zones.ungroup(zone.group!.id),
                "Could not ungroup.",
              )
            }
          >
            <Unlink className="h-4 w-4" /> Ungroup
          </DropdownMenuItem>
        )}
        {zone.capabilities.transport && zone.status === "playing" && (
          <DropdownMenuItem
            onSelect={() =>
              void run(
                () => zones.send(zone.id, { command: "pause" }),
                "Could not pause.",
              )
            }
          >
            Pause this zone
          </DropdownMenuItem>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
