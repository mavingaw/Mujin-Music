// Credit and editorial helpers shared with the mobile app live in the SDK.
export {
  albumFormat,
  creditRoleLabel,
  creditRolesLabel,
  creditsForTrack,
  creditTracksLabel,
  formatAudio,
  formatPartialDate,
  groupCredits,
  lifeSpanFacts,
  paragraphs,
  roleRank,
  roleVerb,
  tagCreditsAsAbout,
  trackCreditLine,
} from "@parson/music-sdk";

/** Link to a person's credits, optionally filtered to one role. */
export function creditHref(artistId: string, role?: string | null) {
  const params = new URLSearchParams({ id: artistId, tab: "credits" });
  if (role) params.set("role", role);
  return `/artist?${params.toString()}`;
}
