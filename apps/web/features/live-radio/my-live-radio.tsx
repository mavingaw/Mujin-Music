"use client";

import type { LiveStation } from "@parson/music-sdk";
import { GripVertical, Loader2, Radio } from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { usePageTitle } from "@/components/app/title-metadata";
import { cn } from "@/lib/cn";
import { LiveRadioHeader } from "./live-radio-browse";
import { moveId } from "./live-radio-model";
import { StationCard } from "./station-ui";
import {
  liveRadioError,
  useMyLiveStations,
  useReorderLiveStations,
} from "./use-live-radio";

function Reorderable({ stations }: { stations: LiveStation[] }) {
  const reorder = useReorderLiveStations();
  const [dragging, setDragging] = useState<number | null>(null);
  const [over, setOver] = useState<number | null>(null);
  const ids = stations.map((station) => station.id);
  const drop = (to: number) => {
    if (dragging === null) return;
    const next = moveId(ids, dragging, to);
    setDragging(null);
    setOver(null);
    if (next !== ids) reorder.mutate(next);
  };
  return (
    <ul
      aria-label="My Live Radio"
      className="grid grid-cols-2 gap-x-4 gap-y-6 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 2xl:grid-cols-6"
    >
      {stations.map((station, index) => (
        <li
          className={cn(
            "relative list-none rounded-xl transition-[opacity,transform]",
            dragging === index && "opacity-40",
            over === index &&
              dragging !== index &&
              "translate-y-1 ring-2 ring-white/30",
          )}
          draggable
          key={station.id}
          onDragEnd={() => {
            setDragging(null);
            setOver(null);
          }}
          onDragOver={(event) => {
            event.preventDefault();
            setOver(index);
          }}
          onDragStart={(event) => {
            event.dataTransfer.effectAllowed = "move";
            setDragging(index);
          }}
          onDrop={(event) => {
            event.preventDefault();
            drop(index);
          }}
        >
          <span
            aria-hidden="true"
            className="absolute -left-1 top-2 z-10 hidden cursor-grab text-white/40 md:block"
            title="Drag to reorder"
          >
            <GripVertical className="h-4 w-4" />
          </span>
          <StationCard asItem={false} station={station} />
        </li>
      ))}
    </ul>
  );
}

/** The listener's saved stations, in their own order. */
export default function MyLiveRadio() {
  const query = useMyLiveStations();
  usePageTitle("My Live Radio");
  const stations = query.data ?? [];
  return (
    <div className="relative mx-auto w-full max-w-310 overflow-x-clip px-5 pb-36 pt-6 text-zinc-50 sm:px-8">
      <LiveRadioHeader
        active="my"
        eyebrow="Your stations"
        title="My Live Radio"
      >
        <p className="mt-3 max-w-xl text-sm leading-6 text-zinc-400">
          Stations you saved from the directory or added by URL. Drag to put
          your favourites first; they play on any zone.
        </p>
      </LiveRadioHeader>
      {query.isPending && (
        <div className="flex justify-center pt-16 text-zinc-500">
          <Loader2 className="h-6 w-6 animate-spin" />
        </div>
      )}
      {query.isError && (
        <p className="text-sm text-zinc-400">
          {liveRadioError(query.error, "My Live Radio could not be loaded.")}
        </p>
      )}
      {query.data && stations.length === 0 && (
        <div className="grid place-items-center rounded-2xl border border-dashed border-white/12 px-6 py-20 text-center">
          <Radio className="h-10 w-10 text-zinc-600" strokeWidth={1.4} />
          <p className="mt-4 text-lg font-semibold text-white">
            No stations yet
          </p>
          <p className="mt-2 max-w-sm text-sm leading-6 text-zinc-400">
            Tap the heart on any station to keep it here, or add your own with
            its stream URL.
          </p>
          <Link
            className="mt-6 inline-flex h-10 items-center rounded-full bg-white px-6 text-sm font-semibold text-black"
            href="/radio"
          >
            Browse Live Radio
          </Link>
        </div>
      )}
      {stations.length > 0 && <Reorderable stations={stations} />}
    </div>
  );
}
