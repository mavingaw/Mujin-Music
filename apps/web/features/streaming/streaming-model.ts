import type {
  LibraryCatalogAlbum,
  StreamingAlbum,
  StreamingAudioFormat,
  StreamingLibraryItem,
  StreamingProviderId,
  StreamingProviderStatus,
} from "@parson/music-sdk";

/** One album tile in the Library, from local files or a streaming service. */
export type LibraryAlbumEntry = {
  key: string;
  id: string;
  name: string;
  artistId: string;
  artistName: string;
  cover: string;
  year: string;
  source: StreamingProviderId | null;
  formatLabel: string | null;
};

/** Roon's "Storage location" focus: everything, local files, or one service's library. */
export type AlbumSourceFilter = "all" | "local" | StreamingProviderId;

export const albumSourceFilters = [
  "all",
  "local",
  "tidal",
  "qobuz",
] as const satisfies readonly AlbumSourceFilter[];

export function parseAlbumSourceFilter(
  value: string | null | undefined,
): AlbumSourceFilter {
  return albumSourceFilters.includes(value as AlbumSourceFilter)
    ? (value as AlbumSourceFilter)
    : "all";
}

export function localAlbumEntry(album: LibraryCatalogAlbum): LibraryAlbumEntry {
  return {
    key: `local:${album.id}`,
    id: album.id,
    name: album.name,
    artistId: album.artistId,
    artistName: album.artistName,
    cover: album.coverPath,
    year: album.releaseYear,
    source: null,
    formatLabel: null,
  };
}

export function streamingAlbumEntry(
  item: StreamingLibraryItem,
): LibraryAlbumEntry {
  const album = item.item as StreamingAlbum | null;
  return {
    key: item.id,
    id: item.id,
    name: item.title,
    artistId: album?.artist?.id ?? "",
    artistName: item.artist_name,
    cover: item.cover_url ?? "",
    year: (item.release_date ?? "").slice(0, 4),
    source: item.provider,
    formatLabel: item.format_label,
  };
}

const collator = new Intl.Collator(undefined, {
  sensitivity: "base",
  numeric: true,
});

const compareEntries = (left: LibraryAlbumEntry, right: LibraryAlbumEntry) =>
  collator.compare(left.artistName, right.artistName) ||
  left.year.localeCompare(right.year) ||
  collator.compare(left.name, right.name);

/**
 * Interleaves streaming albums into the paged local catalog (ordered by
 * artist). While more local pages remain, streaming albums that sort after
 * the last loaded local album wait for the next page, so infinite scrolling
 * never reshuffles tiles already on screen.
 */
export function mergeLibraryAlbums(
  local: LibraryAlbumEntry[],
  streaming: LibraryAlbumEntry[],
  localComplete: boolean,
): LibraryAlbumEntry[] {
  const pending = [...streaming].sort(compareEntries);
  const last = local.at(-1);
  const eligible = localComplete
    ? pending
    : pending.filter((entry) => !last || compareEntries(entry, last) <= 0);
  const merged: LibraryAlbumEntry[] = [];
  let next = 0;
  for (const entry of local) {
    let candidate = eligible[next];
    while (candidate && compareEntries(candidate, entry) < 0) {
      merged.push(candidate);
      candidate = eligible[++next];
    }
    merged.push(entry);
  }
  merged.push(...eligible.slice(next));
  return merged;
}

export type FormatTone = "hi-res" | "lossless" | "lossy";

export function formatTone(
  format:
    | Pick<StreamingAudioFormat, "lossless" | "bit_depth" | "sample_rate">
    | null
    | undefined,
): FormatTone | null {
  if (!format) return null;
  if (!format.lossless) return "lossy";
  return (format.bit_depth ?? 16) > 16 || (format.sample_rate ?? 0) > 48_000
    ? "hi-res"
    : "lossless";
}

/** Tone from a stored badge label such as `24/96 FLAC` or `AAC 320`. */
export function formatToneFromLabel(label: string | null | undefined) {
  if (!label) return null;
  if (/^(aac|mp3|opus|vorbis)\b/i.test(label)) return "lossy";
  if (/^24\b|^24-bit/i.test(label)) return "hi-res";
  return "lossless";
}

export function signedIn(provider: StreamingProviderStatus | undefined) {
  return provider?.status === "signed_in";
}

export function connectedProviders(
  providers: StreamingProviderStatus[] | undefined,
): StreamingProviderStatus[] {
  return (providers ?? []).filter(signedIn);
}

/** Short, human status line for a service row. */
export function providerStatusLine(provider: StreamingProviderStatus): string {
  switch (provider.status) {
    case "signed_in":
      return provider.account_name
        ? `Signed in as ${provider.account_name}`
        : "Signed in";
    case "pending":
      return "Waiting for you to approve the sign-in";
    case "expired":
      return "Session expired. Sign in again.";
    case "error":
      return provider.status_message ?? "Something went wrong";
    default:
      return provider.credentials.configured
        ? "Credentials saved. Not signed in."
        : "Not configured";
  }
}

export function totalDurationLabel(seconds: number) {
  const minutes = Math.max(1, Math.round(seconds / 60));
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest ? `${hours} hr ${rest} min` : `${hours} hr`;
}
