import type { FocusComposer, FocusPerson } from "@parson/music-sdk";

function plural(count: number, one: string, many: string) {
  return `${count.toLocaleString("en-US")} ${count === 1 ? one : many}`;
}

/** "12 compositions · 4 albums" for a composer row or header. */
export function composerSummary(
  composer: Pick<FocusComposer, "workCount" | "albumCount" | "trackCount">,
): string {
  const parts = [plural(composer.workCount, "composition", "compositions")];
  if (composer.albumCount > 0) {
    parts.push(plural(composer.albumCount, "album", "albums"));
  }
  if (composer.trackCount > composer.workCount) {
    parts.push(plural(composer.trackCount, "track", "tracks"));
  }
  return parts.join(" · ");
}

/** Up to two initials for a person without artwork. */
export function composerInitials(name: string): string {
  const words = name
    .replace(/[^\p{L}\p{N}\s-]/gu, " ")
    .split(/[\s-]+/)
    .filter(Boolean);
  if (!words.length) return "?";
  const first = words[0]!;
  const last = words.length > 1 ? words[words.length - 1]! : "";
  return `${first[0] ?? ""}${last[0] ?? ""}`.toUpperCase();
}

const ROLE_LABELS: Record<string, string> = {
  primary: "Artist",
  composer: "Composer",
  lyricist: "Lyricist",
  arranger: "Arranger",
  performer: "Performer",
  conductor_ensemble: "Conductor / Ensemble",
  production: "Production",
};

/** The credit roles line under a person's name, e.g. "Composer · Performer". */
export function personRoles(person: Pick<FocusPerson, "roles">): string {
  return person.roles
    .map((role) => ROLE_LABELS[role.role] ?? role.role)
    .join(" · ");
}
