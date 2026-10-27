import type { Profile } from "@parson/music-sdk";

const plays = new Intl.NumberFormat("en", { notation: "standard" });

const serviceNames: Record<string, string> = {
  lastfm: "Last.fm",
  listenbrainz: "ListenBrainz",
};

/** "1,284 plays · Last.fm · PIN" under a profile's name. */
export function profileSubtitle(profile: Profile): string {
  const parts: string[] = [];
  parts.push(
    profile.listens === 1
      ? "1 play"
      : profile.listens > 0
        ? `${plays.format(profile.listens)} plays`
        : "New listener",
  );
  for (const service of profile.scrobbling) {
    parts.push(serviceNames[service] ?? service);
  }
  if (profile.unlock === "pin") parts.push("PIN");
  else if (profile.unlock === "password") parts.push("Password");
  return parts.join(" · ");
}

/** Role and kind as a short badge, or null for an ordinary account. */
export function profileBadge(profile: Profile): string | null {
  if (profile.role === "admin") return "Admin";
  if (profile.kind === "profile") return "Profile";
  return null;
}

/** What the switcher row says it will do when clicked. */
export function switchHint(profile: Profile): string {
  if (profile.isCurrent) return "Listening now";
  if (profile.unlock === "pin") return "Enter PIN to switch";
  if (profile.unlock === "password") return "Enter password to switch";
  return "Switch instantly";
}
