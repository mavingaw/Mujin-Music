"use client";

import { zoneIconName, type Zone, type ZoneIconName } from "@parson/music-sdk";
import {
  AudioWaveform,
  Bed,
  Briefcase,
  Car,
  CookingPot,
  Disc3,
  Headphones,
  Laptop,
  Monitor,
  Music,
  Radio,
  Smartphone,
  Sofa,
  Speaker,
  Tablet,
  Trees,
  Tv,
  type LucideIcon,
} from "lucide-react";

const ICONS: Record<ZoneIconName, LucideIcon> = {
  speaker: Speaker,
  headphones: Headphones,
  monitor: Monitor,
  laptop: Laptop,
  phone: Smartphone,
  tablet: Tablet,
  tv: Tv,
  radio: Radio,
  turntable: Disc3,
  amplifier: AudioWaveform,
  car: Car,
  bed: Bed,
  sofa: Sofa,
  kitchen: CookingPot,
  office: Briefcase,
  garden: Trees,
  music: Music,
};

export const ZONE_ICON_LABELS: Record<ZoneIconName, string> = {
  speaker: "Speaker",
  headphones: "Headphones",
  monitor: "Screen",
  laptop: "Laptop",
  phone: "Phone",
  tablet: "Tablet",
  tv: "TV",
  radio: "Radio",
  turntable: "Turntable",
  amplifier: "Amplifier",
  car: "Car",
  bed: "Bedroom",
  sofa: "Living room",
  kitchen: "Kitchen",
  office: "Office",
  garden: "Garden",
  music: "Music",
};

export function ZoneIconGlyph({
  name,
  className = "h-4 w-4",
}: {
  name: ZoneIconName;
  className?: string;
}) {
  const Icon = ICONS[name] ?? Music;
  return <Icon aria-hidden className={className} />;
}

/** A zone's icon: its custom icon, else its kind's. */
export function ZoneIcon({
  zone,
  className = "h-4 w-4",
}: {
  zone: Pick<Zone, "icon" | "kind">;
  className?: string;
}) {
  return <ZoneIconGlyph className={className} name={zoneIconName(zone)} />;
}
