import type { AlbumAbout } from "../domain/types";

/**
 * The release type ("Album", "EP", "Single", "Live" ...) for an album's
 * eyebrow, or null. Only what a person edited, the matched MusicBrainz
 * release group or the files' own tags name counts: a type is never guessed
 * from the number of tracks or the running time (one long track is not a
 * single, eleven short songs are not an EP).
 */
export function releaseTypeLabel(
  about: Pick<AlbumAbout, "release_type"> | null | undefined,
): string | null {
  const value = about?.release_type?.value?.trim();
  if (!value) return null;
  if (value.length <= 2 || /^ep$/i.test(value)) return value.toUpperCase();
  return value
    .split(/(\s+|\+)/)
    .map((part) =>
      /^[a-z]/i.test(part)
        ? part.charAt(0).toUpperCase() + part.slice(1).toLowerCase()
        : part,
    )
    .join("");
}
