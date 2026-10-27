"use client";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { QualityBadge } from "@/features/library/compact-track-list";
import { catalogSongsToQueue } from "@/features/library/focus/focus-state";
import SongMenu from "@/features/library/song-menu";
import { formatDuration } from "@/features/library/library-view-primitives";
import { useZonePlayback } from "@/features/player/zones/use-zone-playback";
import getBaseURL from "@/lib/api/server-url";
import { cn } from "@/lib/cn";
import { defaultCover } from "@/lib/images/default-cover";
import { getLibraryImageUrl } from "@/lib/images/image-url";
import {
  HISTORY_PERIODS,
  deleteHistoryEntry,
  exportHistoryCsv,
  formatListeningTime,
  getHistoryFilters,
  getHistoryStats,
  getHistoryTimeline,
  groupHistoryByDay,
  historyPeriod,
  historySourceLabel,
  historyTimeLabel,
  historyZoneLabel,
  localTimezoneOffset,
  periodChange,
  type HistoryEntry,
  type HistoryFilter,
  type HistoryPeriodPreset,
  type HistoryRankedItem,
} from "@parson/music-sdk";
import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import {
  ChevronDown,
  Download,
  History as HistoryIcon,
  Loader2,
  MonitorSpeaker,
  Play,
  Radio,
  Search,
  X,
} from "lucide-react";
import Image from "next/image";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import { ColumnChart, RankedList, StatTile } from "./history-charts";
import {
  bucketLabel,
  bucketTitle,
  clockOrder,
  fillBuckets,
  formatChange,
  labelEvery,
} from "./history-model";

export const HISTORY_KEY = ["history"] as const;
type Tab = "overview" | "timeline";
const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

function entryQueue(entries: HistoryEntry[]) {
  return catalogSongsToQueue(
    entries
      .filter((entry) => entry.available)
      .map((entry) => ({
        id: entry.songId,
        name: entry.title,
        artistId: entry.artistId ?? "",
        artistName: entry.artistName,
        albumId: entry.albumId ?? "",
        albumName: entry.albumName ?? "",
        coverPath: entry.coverPath ?? "",
        path: "",
        durationSeconds: entry.durationSeconds,
      })),
  );
}

function FilterMenu({
  label,
  options,
  value,
  onChange,
  icon,
}: {
  label: string;
  options: { value: string; label: string; count?: number }[];
  value?: string;
  onChange: (value?: string) => void;
  icon: React.ReactNode;
}) {
  const current = options.find((option) => option.value === value);
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          className={cn(
            "flex h-9 max-w-56 items-center gap-1.5 rounded-full border px-3.5 text-sm transition-colors",
            current
              ? "border-white/40 bg-white/10 text-white"
              : "border-white/10 text-zinc-300 hover:border-white/25 hover:text-white",
          )}
          type="button"
        >
          {icon}
          <span className="truncate">{current?.label ?? label}</span>
          <ChevronDown className="h-3.5 w-3.5 shrink-0 opacity-70" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        className="max-h-80 w-60 overflow-y-auto"
      >
        <DropdownMenuRadioGroup
          onValueChange={(next) => onChange(next || undefined)}
          value={value ?? ""}
        >
          <DropdownMenuRadioItem value="">{label}</DropdownMenuRadioItem>
          {options.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value}>
              <span className="min-w-0 flex-1 truncate">{option.label}</span>
              {option.count !== undefined && (
                <span className="ml-3 text-xs tabular-nums text-zinc-500">
                  {option.count}
                </span>
              )}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function Panel({
  title,
  children,
  className,
  action,
}: {
  title: string;
  children: React.ReactNode;
  className?: string;
  action?: React.ReactNode;
}) {
  return (
    <section
      className={cn(
        "rounded-2xl border border-white/8 bg-white/[0.02] p-5",
        className,
      )}
    >
      <div className="mb-4 flex items-center justify-between gap-3">
        <h2 className="text-base font-bold text-white">{title}</h2>
        {action}
      </div>
      {children}
    </section>
  );
}

export default function HistoryPage() {
  const searchParams = useSearchParams();
  const router = useRouter();
  const queryClient = useQueryClient();
  const playback = useZonePlayback();
  const tab: Tab =
    searchParams.get("tab") === "timeline" ? "timeline" : "overview";
  const preset = (
    HISTORY_PERIODS.some(
      (option) => option.value === searchParams.get("period"),
    )
      ? searchParams.get("period")
      : "month"
  ) as HistoryPeriodPreset;
  const zone = searchParams.get("zone") ?? undefined;
  const source = searchParams.get("source") ?? undefined;
  const [search, setSearch] = useState("");
  const [exporting, setExporting] = useState(false);

  const setParam = (name: string, value?: string) => {
    const params = new URLSearchParams(searchParams.toString());
    if (value) params.set(name, value);
    else params.delete(name);
    const query = params.toString();
    router.replace(query ? `/history?${query}` : "/history", { scroll: false });
  };

  // Computed once per preset so the query key stays stable.
  const period = useMemo(() => historyPeriod(preset), [preset]);
  const tz = useMemo(() => localTimezoneOffset(), []);
  const filter: HistoryFilter = {
    from: period.from,
    to: period.to,
    zone,
    source,
    q: search.trim() || undefined,
    tz,
  };

  const filters = useQuery({
    queryKey: [...HISTORY_KEY, "filters"],
    queryFn: getHistoryFilters,
  });
  const stats = useQuery({
    queryKey: [...HISTORY_KEY, "stats", filter, period.bucket],
    queryFn: () => getHistoryStats(filter, { bucket: period.bucket, top: 10 }),
    enabled: tab === "overview",
    placeholderData: (previous) => previous,
  });
  const timeline = useInfiniteQuery({
    queryKey: [...HISTORY_KEY, "timeline", filter],
    queryFn: ({ pageParam }) =>
      getHistoryTimeline(filter, { beforeId: pageParam, limit: 80 }),
    initialPageParam: null as number | null,
    getNextPageParam: (page) => page.nextBeforeId ?? undefined,
    enabled: tab === "timeline",
  });
  const removeEntry = useMutation({
    mutationFn: deleteHistoryEntry,
    onSuccess: async () => {
      toast.success("Removed from history");
      await queryClient.invalidateQueries({ queryKey: HISTORY_KEY });
    },
    onError: () => toast("Could not remove the play."),
  });

  const entries = useMemo(
    () => timeline.data?.pages.flatMap((page) => page.items) ?? [],
    [timeline.data],
  );
  const days = useMemo(() => groupHistoryByDay(entries), [entries]);

  const exportCsv = async () => {
    setExporting(true);
    try {
      const csv = await exportHistoryCsv(filter);
      const url = URL.createObjectURL(
        new Blob([csv], { type: "text/csv;charset=utf-8" }),
      );
      const link = document.createElement("a");
      link.href = url;
      link.download = `parson-history-${preset}.csv`;
      link.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
      const rows = Math.max(0, csv.split("\r\n").filter(Boolean).length - 1);
      toast.success(`Exported ${rows} ${rows === 1 ? "play" : "plays"}`);
    } catch {
      toast("Could not export your history.");
    } finally {
      setExporting(false);
    }
  };

  const playRanked = (item: HistoryRankedItem) => {
    const queue = catalogSongsToQueue([
      {
        id: item.id,
        name: item.name,
        artistId: item.detailId ?? "",
        artistName: item.detail ?? "",
        albumId: "",
        albumName: "",
        coverPath: item.imagePath ?? "",
        path: "",
        durationSeconds: 0,
      },
    ]);
    void playback.playQueue(queue);
  };

  const zoneOptions = (filters.data?.zones ?? []).map((option) => ({
    value: option.value,
    label: historyZoneLabel(option),
    count: option.plays,
  }));
  const sourceOptions = (filters.data?.sources ?? []).map((option) => ({
    value: option.value,
    label: historySourceLabel(option.value),
    count: option.plays,
  }));
  const data = stats.data;
  const chartData = data
    ? fillBuckets(data.timeline, period, data.bucket).map((item) => ({
        key: item.bucket,
        label: bucketLabel(item.bucket, data.bucket),
        title: bucketTitle(item.bucket, data.bucket),
        value: item.plays,
        detail: formatListeningTime(item.listenedSeconds),
      }))
    : [];
  const firstPlay = filters.data?.firstPlay
    ? new Date(filters.data.firstPlay).toLocaleDateString("en-GB", {
        month: "long",
        year: "numeric",
      })
    : null;

  return (
    <div className="relative text-zinc-50">
      <div className="pointer-events-none absolute inset-x-0 top-0 h-80 bg-linear-to-b from-mujin-copper/12 via-mujin-copper/[0.03] to-transparent" />
      <div className="relative mx-auto max-w-6xl px-5 pb-24 pt-10 sm:px-8">
        <header className="mb-7 flex flex-wrap items-end justify-between gap-5">
          <div>
            <p className="mb-3 flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.14em] text-zinc-500">
              <HistoryIcon className="h-3.5 w-3.5" /> Browse
            </p>
            <h1 className="text-4xl mj-display text-mujin-text sm:text-5xl">
              History
            </h1>
            <p className="mt-3 text-sm text-zinc-400">
              {filters.isPending
                ? "Loading…"
                : filters.data?.total
                  ? `${filters.data.total.toLocaleString("en-GB")} plays${
                      firstPlay ? ` since ${firstPlay}` : ""
                    } · just your profile`
                  : "Every track you play, on every zone, kept here."}
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <button
              className="flex h-9 items-center gap-1.5 rounded-full border border-white/10 px-3.5 text-sm text-zinc-300 hover:border-white/25 hover:text-white disabled:opacity-50"
              disabled={exporting}
              onClick={() => void exportCsv()}
              type="button"
            >
              {exporting ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Download className="h-4 w-4" />
              )}
              Export CSV
            </button>
            <div className="flex items-center gap-1 rounded-full border border-white/10 p-0.5">
              {(["overview", "timeline"] as Tab[]).map((option) => (
                <button
                  aria-pressed={tab === option}
                  className={cn(
                    "h-9 rounded-full px-4 text-sm font-semibold capitalize transition-colors",
                    tab === option
                      ? "bg-white text-black"
                      : "text-zinc-400 hover:text-white",
                  )}
                  key={option}
                  onClick={() =>
                    setParam("tab", option === "overview" ? undefined : option)
                  }
                  type="button"
                >
                  {option}
                </button>
              ))}
            </div>
          </div>
        </header>

        <div className="mb-8 flex flex-wrap items-center gap-2" role="toolbar">
          <div className="flex flex-wrap rounded-full border border-white/10 p-0.5">
            {HISTORY_PERIODS.map((option) => (
              <button
                aria-pressed={preset === option.value}
                className={cn(
                  "h-8 rounded-full px-3 text-sm transition-colors",
                  preset === option.value
                    ? "bg-white/12 font-semibold text-white"
                    : "text-zinc-400 hover:text-white",
                )}
                key={option.value}
                onClick={() =>
                  setParam(
                    "period",
                    option.value === "month" ? undefined : option.value,
                  )
                }
                type="button"
              >
                {option.label}
              </button>
            ))}
          </div>
          <FilterMenu
            icon={<MonitorSpeaker className="h-4 w-4" />}
            label="All zones"
            onChange={(value) => setParam("zone", value)}
            options={zoneOptions}
            value={zone}
          />
          <FilterMenu
            icon={<Radio className="h-4 w-4" />}
            label="All sources"
            onChange={(value) => setParam("source", value)}
            options={sourceOptions}
            value={source}
          />
          <label className="flex h-9 w-full items-center gap-2 rounded-full border border-white/10 px-3.5 text-sm focus-within:border-white/30 sm:w-52">
            <Search className="h-4 w-4 text-zinc-500" />
            <input
              aria-label="Search history"
              className="min-w-0 flex-1 bg-transparent text-white outline-none placeholder:text-zinc-500"
              onChange={(event) => setSearch(event.target.value)}
              placeholder="Track, artist, album"
              value={search}
            />
          </label>
        </div>

        {tab === "overview" ? (
          stats.isPending ? (
            <div className="flex justify-center pt-16 text-zinc-500">
              <Loader2 className="h-6 w-6 animate-spin" />
            </div>
          ) : stats.isError || !data ? (
            <p className="pt-10 text-center text-sm text-zinc-400">
              Could not load your listening stats.
            </p>
          ) : data.totals.plays === 0 ? (
            <div className="mx-auto mt-6 max-w-md rounded-2xl border border-white/8 bg-white/2 px-8 py-12 text-center">
              <div className="mx-auto mb-5 grid h-16 w-16 place-items-center rounded-full bg-mujin-copper/15 text-mujin-copper-hover">
                <HistoryIcon className="h-8 w-8" />
              </div>
              <h2 className="text-xl font-bold text-white">
                No plays here yet
              </h2>
              <p className="mt-2 text-sm leading-6 text-zinc-400">
                A track lands in your history once you have heard half of it (or
                four minutes). Try a longer period or clear the filters.
              </p>
            </div>
          ) : (
            <div className={cn("space-y-5", stats.isFetching && "opacity-80")}>
              <div className="grid grid-cols-2 gap-3 md:grid-cols-3 lg:grid-cols-6">
                <StatTile
                  change={formatChange(
                    periodChange(data.totals.plays, data.previous?.plays),
                  )}
                  label="Plays"
                  value={data.totals.plays.toLocaleString("en-GB")}
                />
                <StatTile
                  change={formatChange(
                    periodChange(
                      data.totals.listenedSeconds,
                      data.previous?.listenedSeconds,
                    ),
                  )}
                  label="Listening"
                  value={formatListeningTime(data.totals.listenedSeconds)}
                />
                <StatTile
                  hint="different artists"
                  label="Artists"
                  value={String(data.totals.artists)}
                />
                <StatTile
                  hint="different albums"
                  label="Albums"
                  value={String(data.totals.albums)}
                />
                <StatTile
                  hint="different tracks"
                  label="Tracks"
                  value={String(data.totals.tracks)}
                />
                <StatTile
                  hint="with music"
                  label="Days"
                  value={String(data.totals.activeDays)}
                />
              </div>

              <Panel title="Plays over time">
                <ColumnChart
                  ariaLabel="Plays over time"
                  data={chartData}
                  labelEvery={labelEvery(chartData.length)}
                />
              </Panel>

              <div className="grid gap-5 lg:grid-cols-2">
                <Panel title="Top artists">
                  <RankedList items={data.topArtists} kind="artist" />
                </Panel>
                <Panel title="Top albums">
                  <RankedList items={data.topAlbums} kind="album" />
                </Panel>
              </div>
              <div className="grid gap-5 lg:grid-cols-5">
                <Panel className="lg:col-span-3" title="Top tracks">
                  <RankedList
                    items={data.topTracks}
                    kind="track"
                    onPlay={playRanked}
                  />
                </Panel>
                <div className="space-y-5 lg:col-span-2">
                  <Panel title="When you listen">
                    <ColumnChart
                      ariaLabel="Plays by hour of day"
                      data={clockOrder(data.hours).map(({ hour, plays }) => ({
                        key: String(hour),
                        label: String(hour).padStart(2, "0"),
                        title: `${String(hour).padStart(2, "0")}:00–${String((hour + 1) % 24).padStart(2, "0")}:00`,
                        value: plays,
                      }))}
                      height={96}
                      labelEvery={6}
                    />
                    <div className="mt-4">
                      <ColumnChart
                        ariaLabel="Plays by weekday"
                        data={WEEKDAYS.map((day, index) => ({
                          key: day,
                          label: day,
                          title: day,
                          value: data.weekdays[index] ?? 0,
                        }))}
                        height={72}
                      />
                    </div>
                  </Panel>
                  <Panel title="Zones">
                    <ul className="space-y-2">
                      {data.zones.map((item) => {
                        const share =
                          item.plays / Math.max(1, data.totals.plays);
                        return (
                          <li key={item.value}>
                            <button
                              className="w-full text-left"
                              onClick={() => setParam("zone", item.value)}
                              type="button"
                            >
                              <span className="flex items-baseline justify-between gap-3 text-sm">
                                <span className="truncate text-zinc-100">
                                  {historyZoneLabel(item)}
                                </span>
                                <span className="shrink-0 text-xs tabular-nums text-zinc-500">
                                  {item.plays} · {Math.round(share * 100)}%
                                </span>
                              </span>
                              <span className="mt-1 block h-1.5 overflow-hidden rounded-full bg-white/[0.05]">
                                <span
                                  className="block h-full rounded-full bg-mujin-copper/80"
                                  style={{ width: `${share * 100}%` }}
                                />
                              </span>
                            </button>
                          </li>
                        );
                      })}
                    </ul>
                  </Panel>
                </div>
              </div>
            </div>
          )
        ) : timeline.isPending ? (
          <div className="flex justify-center pt-16 text-zinc-500">
            <Loader2 className="h-6 w-6 animate-spin" />
          </div>
        ) : timeline.isError ? (
          <p className="pt-10 text-center text-sm text-zinc-400">
            Could not load your history.
          </p>
        ) : !days.length ? (
          <p className="pt-10 text-center text-sm text-zinc-500">
            No plays match these filters.
          </p>
        ) : (
          <div className="space-y-8">
            {days.map((day) => (
              <section key={day.key}>
                <div className="sticky top-0 z-10 -mx-2 mb-2 flex items-baseline justify-between gap-3 bg-mujin-base/90 px-2 py-2 backdrop-blur">
                  <h2 className="text-lg font-bold text-white">{day.label}</h2>
                  <span className="text-xs tabular-nums text-zinc-500">
                    {day.items.length}{" "}
                    {day.items.length === 1 ? "play" : "plays"} ·{" "}
                    {formatListeningTime(day.listenedSeconds)}
                  </span>
                </div>
                <ol className="divide-y divide-white/[0.04] overflow-hidden rounded-xl border border-white/8 bg-white/[0.015]">
                  {day.items.map((entry) => (
                    <TimelineRow
                      entry={entry}
                      key={entry.id}
                      onPlay={() => {
                        const queue = entryQueue(day.items);
                        const index = queue.findIndex(
                          (item) => item.song.id === entry.songId,
                        );
                        void playback.playQueue(queue, Math.max(0, index));
                      }}
                      onRemove={() => removeEntry.mutate(entry.id)}
                      onZone={(value) => setParam("zone", value)}
                    />
                  ))}
                </ol>
              </section>
            ))}
            {timeline.hasNextPage && (
              <div className="flex justify-center">
                <button
                  className="flex h-10 items-center gap-2 rounded-full border border-white/12 px-5 text-sm font-semibold text-zinc-200 hover:border-white/30 hover:text-white disabled:opacity-60"
                  disabled={timeline.isFetchingNextPage}
                  onClick={() => void timeline.fetchNextPage()}
                  type="button"
                >
                  {timeline.isFetchingNextPage && (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  )}
                  Load earlier plays
                </button>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function TimelineRow({
  entry,
  onPlay,
  onRemove,
  onZone,
}: {
  entry: HistoryEntry;
  onPlay: () => void;
  onRemove: () => void;
  onZone: (zone: string) => void;
}) {
  const cover = entry.coverPath
    ? (getLibraryImageUrl(entry.coverPath, getBaseURL) ?? defaultCover)
    : defaultCover;
  const row = (
    <li className="group flex items-center gap-3 px-3 py-2.5 transition-colors hover:bg-white/[0.04]">
      <span className="w-11 shrink-0 text-xs tabular-nums text-zinc-500">
        {historyTimeLabel(entry.listenedAt)}
      </span>
      <button
        aria-label={`Play ${entry.title}`}
        className="relative h-10 w-10 shrink-0 overflow-hidden rounded-md bg-zinc-900 disabled:cursor-default"
        disabled={!entry.available}
        onClick={onPlay}
        type="button"
      >
        <Image
          alt=""
          className="h-full w-full object-cover"
          height={80}
          src={cover}
          width={80}
        />
        {entry.available && (
          <span className="absolute inset-0 grid place-items-center bg-black/55 opacity-0 transition-opacity group-hover:opacity-100">
            <Play className="h-4 w-4 fill-white text-white" />
          </span>
        )}
      </button>
      <div className="min-w-0 flex-1">
        <p className="flex items-center gap-2 truncate text-sm font-medium text-zinc-100">
          <span className={cn("truncate", !entry.available && "text-zinc-400")}>
            {entry.title}
          </span>
          <QualityBadge quality={entry.quality} />
          {!entry.available && (
            <span className="shrink-0 text-[0.6875rem] text-zinc-600">
              not in library
            </span>
          )}
        </p>
        <p className="truncate text-xs text-zinc-500">
          {entry.artistId && entry.available ? (
            <Link
              className="hover:text-white hover:underline"
              href={`/artist?id=${encodeURIComponent(entry.artistId)}`}
            >
              {entry.artistName}
            </Link>
          ) : (
            entry.artistName
          )}
          {entry.albumName && (
            <>
              {" · "}
              {entry.albumId && entry.available ? (
                <Link
                  className="hover:text-white hover:underline"
                  href={`/album?id=${encodeURIComponent(entry.albumId)}`}
                >
                  {entry.albumName}
                </Link>
              ) : (
                entry.albumName
              )}
            </>
          )}
        </p>
      </div>
      <div className="hidden shrink-0 items-center gap-1.5 md:flex">
        {entry.zoneName && (
          <button
            className="flex max-w-40 items-center gap-1 rounded-full border border-white/10 px-2 py-0.5 text-[0.6875rem] text-zinc-300 hover:border-white/25 hover:text-white"
            onClick={() => entry.zoneId && onZone(entry.zoneId)}
            title={
              entry.device
                ? `${entry.zoneName} · ${entry.device}`
                : entry.zoneName
            }
            type="button"
          >
            <MonitorSpeaker className="h-3 w-3 shrink-0" />
            <span className="truncate">{entry.zoneName}</span>
          </button>
        )}
        {entry.source === "generated" && (
          <span className="flex items-center gap-1 rounded-full border border-mujin-copper/20 px-2 py-0.5 text-[0.6875rem] text-mujin-copper-hover">
            <Radio className="h-3 w-3" /> Radio
          </span>
        )}
      </div>
      <span className="w-11 shrink-0 text-right text-xs tabular-nums text-zinc-500">
        {entry.durationSeconds > 0 ? formatDuration(entry.durationSeconds) : ""}
      </span>
      <button
        aria-label={`Remove this play of ${entry.title} from history`}
        className="grid h-7 w-7 shrink-0 place-items-center rounded-full text-zinc-500 opacity-0 transition-opacity hover:text-white focus-visible:opacity-100 group-hover:opacity-100"
        onClick={onRemove}
        title="Remove from history"
        type="button"
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </li>
  );
  return entry.available ? (
    <SongMenu
      album_cover={entry.coverPath ?? undefined}
      album_id={entry.albumId ?? ""}
      album_name={entry.albumName ?? ""}
      artist_id={entry.artistId ?? ""}
      artist_name={entry.artistName}
      song_id={entry.songId}
      song_name={entry.title}
    >
      {row}
    </SongMenu>
  ) : (
    row
  );
}
