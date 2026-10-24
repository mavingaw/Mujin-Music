"use client";

import {
  formatSleepRemaining,
  groupingPlan,
  orderZones,
  zoneKindLabel,
  zoneStatusLabel,
  type Zone,
  type ZoneKind,
} from "@parson/music-sdk";
import {
  ArrowRightLeft,
  Cast,
  Check,
  Laptop,
  Link2,
  Monitor,
  Moon,
  MoreHorizontal,
  RefreshCw,
  Settings2,
  Smartphone,
  Speaker,
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
import { Input } from "@/components/ui/input";
import { Slider } from "@/components/ui/slider";
import { useZones } from "./zones-provider";

const SLEEP_CHOICES = [15, 30, 45, 60, 90] as const;

export function ZoneIcon({
  kind,
  className = "h-4 w-4",
}: {
  kind: ZoneKind;
  className?: string;
}) {
  switch (kind) {
    case "mobile":
      return <Smartphone className={className} />;
    case "desktop":
      return <Laptop className={className} />;
    case "chromecast":
      return <Cast className={className} />;
    case "sonos":
      return <Speaker className={className} />;
    case "linked":
      return <Link2 className={className} />;
    default:
      return <Monitor className={className} />;
  }
}

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
    end_of_queue: "That was the last track in the queue.",
    sonos_disabled: "Sonos support is turned off on this server.",
  };
  return known[code] ?? code.replaceAll("_", " ");
}

/** The footer zone button (Roon's zone picker). */
export function ZonePickerButton({ compact = false }: { compact?: boolean }) {
  const { zones, remoteZone, localZoneId } = useZones();
  const [open, setOpen] = useState(false);
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
        type="button"
      >
        <ZoneIcon kind={current?.kind ?? "web"} />
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
                onEdit={(value) => setEditing(value ? zone.id : null)}
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
                        ? "Grouped. Rooms play in sync."
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
  const { sonos } = useZones();
  if (!sonos) return null;
  return (
    <p className="border-t border-white/10 px-5 py-3 text-[11px] leading-4 text-zinc-500">
      {sonos.enabled
        ? sonos.players
          ? `${sonos.players} Sonos ${sonos.players === 1 ? "room" : "rooms"} on this network. Stereo pairs count as one room.`
          : "No Sonos found. Sonos must be on the same network as the Parson server, and the server must accept connections from it (UDP 1900 for discovery and the server port for audio)."
        : "Sonos support is turned off on this server (PARSON_SONOS=off)."}
    </p>
  );
}

function ZoneRow({
  active,
  checked,
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
  editing: boolean;
  grouping: boolean;
  isLocal: boolean;
  onCheck: (value: boolean) => void;
  onEdit: (value: boolean) => void;
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
          disabled={grouping}
          onClick={onSelect}
          type="button"
        >
          <span
            className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-lg ${zone.status === "playing" ? "bg-emerald-400/15 text-emerald-300" : "bg-white/[0.06] text-zinc-400"}`}
          >
            <ZoneIcon kind={zone.kind} />
          </span>
          <span className="min-w-0 flex-1">
            <span className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
              <span className="max-w-full truncate text-sm font-medium text-white">
                {zone.name}
              </span>
              {isLocal && <Badge>This device</Badge>}
              {zone.group &&
                (zone.group.synced ? (
                  <Badge tone="synced">Synced group</Badge>
                ) : (
                  <Badge tone="linked">Linked · not sample-synced</Badge>
                ))}
              {active && <Check className="h-3.5 w-3.5 text-white" />}
            </span>
            <span className="block truncate text-xs text-zinc-400">
              {subtitle}
            </span>
            <span className="block truncate text-[11px] text-zinc-600">
              {device}
              {zone.sleep_timer
                ? ` · Sleep in ${formatSleepRemaining(zone.sleep_timer.ends_at_ms, Date.now())}${zone.sleep_timer.finishing_track ? " (finishing track)" : ""}`
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
          <ZoneMenu onEdit={() => onEdit(!editing)} run={run} zone={zone} />
        )}
      </div>
      {!grouping && zone.volume && zone.capabilities.volume && (
        <ZoneVolumeRow
          label={zone.group ? "Group volume" : "Volume"}
          muted={zone.volume.muted}
          max={zone.volume.max}
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
                    max={member.volume.max}
                    muted={member.volume.muted}
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
      {editing && <ZoneSettingsForm onDone={() => onEdit(false)} zone={zone} />}
    </li>
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
  onVolume,
  volume,
}: {
  dense?: boolean;
  label: string;
  max: number;
  muted: boolean;
  onMute: (muted: boolean) => void;
  onVolume: (volume: number) => void;
  volume: number;
}) {
  const [draft, setDraft] = useState<number | null>(null);
  useEffect(() => setDraft(null), [volume]);
  const shown = draft ?? volume;
  return (
    <div className={`flex items-center gap-2 ${dense ? "" : "mt-2 pl-12"}`}>
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
      <Slider
        aria-label={label}
        className="h-7 flex-1 **:data-[slot=slider-range]:bg-white **:data-[slot=slider-thumb]:h-3 **:data-[slot=slider-thumb]:w-3 **:data-[slot=slider-thumb]:border-0 **:data-[slot=slider-thumb]:bg-white **:data-[slot=slider-track]:h-1 **:data-[slot=slider-track]:bg-white/15"
        max={100}
        min={0}
        onValueChange={([value]) => setDraft(Math.min(value ?? 0, max))}
        onValueCommit={([value]) => onVolume(Math.min(value ?? 0, max))}
        value={[muted ? 0 : shown]}
      />
      <span className="w-12 text-right text-[11px] tabular-nums text-zinc-500">
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
  onEdit: () => void;
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
        <DropdownMenuItem onSelect={onEdit}>
          <Settings2 className="h-4 w-4" /> Zone settings
        </DropdownMenuItem>
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

function ZoneSettingsForm({
  onDone,
  zone,
}: {
  onDone: () => void;
  zone: Zone;
}) {
  const zones = useZones();
  const [name, setName] = useState(zone.name);
  const [limit, setLimit] = useState(zone.volume?.max ?? 100);
  return (
    <form
      className="mt-3 space-y-3 rounded-lg border border-white/10 p-3"
      onSubmit={(event) => {
        event.preventDefault();
        void zones
          .updateSettings(zone.id, {
            name: name.trim() && name.trim() !== zone.name ? name.trim() : null,
            max_volume: limit < 100 ? limit : null,
          })
          .then(() => {
            toast.success("Zone settings saved");
            onDone();
          })
          .catch((error) =>
            toast.error(errorMessage(error, "Could not save settings.")),
          );
      }}
    >
      <label className="block text-xs text-zinc-400">
        Zone name
        <Input
          className="mt-1 h-9"
          maxLength={80}
          onChange={(event) => setName(event.target.value)}
          value={name}
        />
      </label>
      <label className="block text-xs text-zinc-400">
        Volume limit · {limit}
        <Slider
          aria-label="Volume limit"
          className="mt-2 h-6"
          max={100}
          min={1}
          onValueChange={([value]) => setLimit(value ?? 100)}
          value={[limit]}
        />
        <span className="mt-1 block text-[11px] text-zinc-600">
          Remote and in-app volume never goes above this level.
        </span>
      </label>
      <div className="flex justify-end gap-2">
        <button
          className="h-8 rounded-full px-3 text-xs text-zinc-400 hover:text-white"
          onClick={onDone}
          type="button"
        >
          Cancel
        </button>
        <button
          className="h-8 rounded-full bg-white px-3 text-xs font-semibold text-black"
          type="submit"
        >
          Save
        </button>
      </div>
    </form>
  );
}
