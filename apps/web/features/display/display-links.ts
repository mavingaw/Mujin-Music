"use client";

import getBaseURL from "@/lib/api/server-url";
import { createDisplay, displayLink, getDisplays } from "@parson/music-sdk";
import { readStored, writeStored } from "./display-model";

/**
 * Display links this browser made, per zone ("follow" for the one that
 * follows whatever plays), so opening a zone's display again reuses its
 * display instead of adding another to Settings → Displays.
 */
const LINKS_KEY = "mujin:display-links";

type StoredLinks = Record<string, { id: string; key: string }>;

function storedLinks(): StoredLinks {
  try {
    const parsed = JSON.parse(readStored(LINKS_KEY) ?? "{}") as unknown;
    return parsed && typeof parsed === "object" ? (parsed as StoredLinks) : {};
  } catch {
    return {};
  }
}

/** The link that shows `zone` (or whatever plays, for `null`) full screen. */
export async function displayLinkForZone(
  zone: { id: string; name: string } | null,
): Promise<string> {
  const slot = zone?.id ?? "follow";
  const links = storedLinks();
  const known = links[slot];
  if (known) {
    const displays = await getDisplays().catch(() => null);
    if (displays?.some((display) => display.id === known.id)) {
      return displayLink(getBaseURL(), known.key);
    }
  }
  const created = await createDisplay({
    name: zone ? `${zone.name} display` : "Now playing display",
    zone_id: zone?.id ?? null,
  });
  writeStored(
    LINKS_KEY,
    JSON.stringify({
      ...links,
      [slot]: { id: created.display.id, key: created.key },
    }),
  );
  return displayLink(getBaseURL(), created.key);
}

/**
 * Opens a zone's display in a new window. The window opens at once (inside
 * the click, so no pop-up blocker stops it) and goes to the link when it
 * is ready; without a window the link opens here.
 */
export async function openZoneDisplay(
  zone: { id: string; name: string } | null,
): Promise<void> {
  const target = window.open("about:blank", "_blank");
  try {
    const link = await displayLinkForZone(zone);
    if (target) {
      target.opener = null;
      target.location.href = link;
    } else {
      window.location.href = link;
    }
  } catch (error) {
    target?.close();
    throw error;
  }
}
