import api from "../core/http";
import {
  LibrarySong,
  BareSong,
  LibraryMetadataPatch,
  LibraryMetadataResponse,
} from "../domain/types";
import {
  type PlaybackCapabilities,
  playbackCapabilityParams,
} from "../player/formats";
import type {
  ServerSignalPath,
  SignalStreamTarget,
} from "../player/signal-path";
import { replaceCachedAlbumInfo } from "./albums";
import { replaceCachedArtistInfo } from "./artists";

type SongInfoValue = LibrarySong | BareSong;
type SongInfoResponse = { Full?: LibrarySong; Bare?: BareSong };

const MAX_BATCH_LOOKUP_IDS = 500;
const MAX_PARALLEL_BATCHES = 4;
const SONG_INFO_CACHE_TTL_MS = 5 * 60 * 1000;
const songInfoCache = new Map<
  string,
  { expiresAt: number; value: SongInfoValue }
>();
const songInfoRequests = new Map<string, Promise<SongInfoValue>>();

const songInfoCacheKey = (id: string, bare: boolean) => `${id}:${bare}`;

const unwrapSongInfoResponse = (data: SongInfoResponse): SongInfoValue => {
  if (data.Full) {
    return data.Full;
  }
  if (data.Bare) {
    return data.Bare;
  }
  throw new Error("Unexpected response format");
};

const readCachedSongInfo = (id: string, bare: boolean) => {
  const key = songInfoCacheKey(id, bare);
  const cached = songInfoCache.get(key);
  if (!cached) return null;

  if (cached.expiresAt <= Date.now()) {
    songInfoCache.delete(key);
    return null;
  }

  return cached.value;
};

const writeCachedSongInfo = (
  id: string,
  bare: boolean,
  value: SongInfoValue,
) => {
  songInfoCache.set(songInfoCacheKey(id, bare), {
    expiresAt: Date.now() + SONG_INFO_CACHE_TTL_MS,
    value,
  });
};

const clearSongInfoCacheForId = (id: string) => {
  songInfoCache.delete(songInfoCacheKey(id, true));
  songInfoCache.delete(songInfoCacheKey(id, false));
};

export function clearCachedSongInfos() {
  songInfoCache.clear();
}

export async function getRandomSong(
  amount: number,
  genre?: string,
): Promise<LibrarySong[]> {
  const params = genre ? { genre } : {};
  const response = await api.get<LibrarySong[]>(`/songs/random/${amount}`, {
    params,
  });
  return response.data;
}

export async function editAlbumMetadata(
  albumId: string,
  metadata: Omit<LibraryMetadataPatch, "song">,
): Promise<import("../domain/types").AlbumMetadataResponse> {
  const response = await api.post<
    import("../domain/types").AlbumMetadataResponse
  >(`/metadata/album/${encodeURIComponent(albumId)}`, metadata);
  replaceCachedAlbumInfo(albumId, response.data.album);
  replaceCachedArtistInfo(response.data.artist.id, response.data.artist);
  return response.data;
}

export async function uploadAlbumCover(
  albumId: string,
  cover: File,
): Promise<string> {
  const formData = new FormData();
  formData.append("cover", cover);
  const response = await api.put<{ cover_url: string }>(
    `/metadata/album/${encodeURIComponent(albumId)}/cover`,
    formData,
  );
  return response.data.cover_url;
}

export async function getLatestSongs(
  amount: number = 20,
): Promise<LibrarySong[]> {
  const response = await api.get<LibrarySong[]>(`/songs/latest`, {
    params: { amount },
  });
  return response.data;
}

export function getSongInfo(id: string, bare: false): Promise<LibrarySong>;
export function getSongInfo(id: string, bare?: true): Promise<BareSong>;
export async function getSongInfo(
  id: string,
  bare: boolean = true,
): Promise<LibrarySong | BareSong> {
  const cached = readCachedSongInfo(id, bare);
  if (cached) return cached;

  const key = songInfoCacheKey(id, bare);
  const existingRequest = songInfoRequests.get(key);
  if (existingRequest) return existingRequest;

  const request = api
    .get<SongInfoResponse>(`/songs/${id}`, {
      params: { bare },
    })
    .then((response) => {
      const song = unwrapSongInfoResponse(response.data);
      writeCachedSongInfo(id, bare, song);
      return song;
    })
    .finally(() => {
      songInfoRequests.delete(key);
    });

  songInfoRequests.set(key, request);
  return request;
}

/**
 * Get multiple songs by ID using the backend batch endpoint. Results are keyed by song ID.
 * Cached and in-flight individual song requests are reused automatically.
 */
export async function getSongInfos(
  ids: string[],
  bare: boolean = true,
): Promise<Record<string, SongInfoValue>> {
  const uniqueIds = Array.from(new Set(ids.filter(Boolean)));
  const results: Record<string, SongInfoValue> = {};
  const missingIds: string[] = [];
  const pendingEntries: Array<[string, Promise<SongInfoValue>]> = [];

  for (const id of uniqueIds) {
    const cached = readCachedSongInfo(id, bare);
    if (cached) {
      results[id] = cached;
    } else {
      const pending = songInfoRequests.get(songInfoCacheKey(id, bare));
      if (pending) pendingEntries.push([id, pending]);
      else missingIds.push(id);
    }
  }

  const resolvePending = async () => {
    const values = await Promise.all(
      pendingEntries.map(async ([id, pending]) => [id, await pending] as const),
    );
    for (const [id, value] of values) results[id] = value;
  };

  if (missingIds.length === 0) {
    await resolvePending();
    return results;
  }

  for (
    let offset = 0;
    offset < missingIds.length;
    offset += MAX_BATCH_LOOKUP_IDS * MAX_PARALLEL_BATCHES
  ) {
    const window = missingIds.slice(
      offset,
      offset + MAX_BATCH_LOOKUP_IDS * MAX_PARALLEL_BATCHES,
    );
    const responses = await Promise.all(
      Array.from(
        { length: Math.ceil(window.length / MAX_BATCH_LOOKUP_IDS) },
        (_, index) =>
          api.post<Record<string, SongInfoResponse>>("/songs/batch", {
            ids: window.slice(
              index * MAX_BATCH_LOOKUP_IDS,
              (index + 1) * MAX_BATCH_LOOKUP_IDS,
            ),
            bare,
          }),
      ),
    );
    for (const response of responses) {
      for (const [id, data] of Object.entries(response.data)) {
        const song = unwrapSongInfoResponse(data);
        writeCachedSongInfo(id, bare, song);
        results[id] = song;
      }
    }
  }

  await resolvePending();

  return results;
}

export async function editLibraryMetadata(
  songId: string,
  metadata: LibraryMetadataPatch,
): Promise<LibraryMetadataResponse> {
  const response = await api.post<LibraryMetadataResponse>(
    `/metadata/song/${encodeURIComponent(songId)}`,
    metadata,
  );
  clearSongInfoCacheForId(songId);
  return response.data;
}

export type SongSignalPathOptions = {
  /** Streaming bitrate in kbps, 0 for the original file (as for streams). */
  bitrate?: number;
  /** Server DSP, as accepted by the stream endpoint. */
  audioEffect?: string | null;
  /**
   * `cast` describes what a Cast receiver is sent. What the server plays
   * itself (devices, bridges, network players) is a zone's
   * `output.stream.signal_path`.
   */
  target?: Exclude<SignalStreamTarget, "native">;
  /**
   * What the player decodes, exactly as sent with its stream request, so
   * the server describes the delivery it will actually make.
   */
  capabilities?: PlaybackCapabilities | null;
};

const signalPathCache = new Map<
  string,
  { expiresAt: number; value: Promise<ServerSignalPath> }
>();
const SIGNAL_PATH_CACHE_TTL_MS = 60 * 1000;
const MAX_SIGNAL_PATH_CACHE_ENTRIES = 64;

/**
 * Describes the server leg of the signal path for a stream requested with
 * the same parameters: source format, DSP, conversions and transcoding.
 */
export function getSongSignalPath(
  songId: string,
  options: SongSignalPathOptions = {},
): Promise<ServerSignalPath> {
  const bitrate = options.bitrate ?? 0;
  const target = options.target ?? "direct";
  const capabilityParams =
    target === "direct" ? playbackCapabilityParams(options.capabilities) : {};
  const key = `${songId}:${bitrate}:${options.audioEffect ?? ""}:${target}:${new URLSearchParams(capabilityParams).toString()}`;
  const cached = signalPathCache.get(key);
  if (cached && cached.expiresAt > Date.now()) return cached.value;
  const params: Record<string, string | number> = {
    ...capabilityParams,
    bitrate,
    target,
  };
  if (options.audioEffect) params.audio_effect = options.audioEffect;
  const value = api
    .get<ServerSignalPath>(
      `/media/songs/${encodeURIComponent(songId)}/signal-path`,
      { params },
    )
    .then((response) => response.data);
  value.catch(() => signalPathCache.delete(key));
  if (signalPathCache.size >= MAX_SIGNAL_PATH_CACHE_ENTRIES) {
    const oldest = signalPathCache.keys().next().value;
    if (oldest !== undefined) signalPathCache.delete(oldest);
  }
  signalPathCache.set(key, {
    expiresAt: Date.now() + SIGNAL_PATH_CACHE_TTL_MS,
    value,
  });
  return value;
}

export function clearCachedSignalPaths() {
  signalPathCache.clear();
}
