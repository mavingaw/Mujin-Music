import api from "../core/http";
import type {
  AlbumIdentification,
  ComposerWorks,
  IdentifySearch,
  ReleaseMatch,
  TrackWork,
  WorkPage,
} from "../domain/types";

const segment = (id: string) => encodeURIComponent(id);

/** A work with every recording of it in the library. */
export async function getWork(mbid: string): Promise<WorkPage> {
  const response = await api.get<WorkPage>(`/works/${segment(mbid)}`);
  return response.data;
}

/** The compositions of a composer that have recordings in the library. */
export async function getArtistWorks(id: string): Promise<ComposerWorks> {
  const response = await api.get<ComposerWorks>(
    `/artists/${segment(id)}/works`,
  );
  return response.data;
}

/** The works a track performs. */
export async function getSongWorks(id: string): Promise<TrackWork[]> {
  const response = await api.get<{ song_id: string; works: TrackWork[] }>(
    `/songs/${segment(id)}/works`,
  );
  return response.data.works;
}

export interface ReleaseSearchTerms {
  /** A raw MusicBrainz search (Lucene syntax), used as typed. */
  query?: string;
  /** Or a title and artist; each defaults to the album's own. An empty
   * artist searches by title alone. */
  title?: string;
  artist?: string;
}

/** Administrator: searches MusicBrainz releases for an album. */
export async function searchAlbumReleases(
  albumId: string,
  terms: ReleaseSearchTerms = {},
): Promise<IdentifySearch> {
  const params = new URLSearchParams({ album_id: albumId });
  const query = terms.query?.trim();
  if (query) params.set("query", query);
  const title = terms.title?.trim();
  if (title) params.set("title", title);
  if (terms.artist !== undefined) params.set("artist", terms.artist.trim());
  const response = await api.get<IdentifySearch>(
    `/metadata/identify?${params.toString()}`,
  );
  return response.data;
}

/** Administrator: pins an album to a MusicBrainz release and looks it up
 * again. */
export async function identifyAlbum(
  albumId: string,
  release: Pick<ReleaseMatch, "mbid" | "title" | "artist">,
): Promise<AlbumIdentification | null> {
  const response = await api.put<{
    identification: AlbumIdentification | null;
  }>(`/metadata/identify/${segment(albumId)}`, {
    release_mbid: release.mbid,
    title: release.title,
    artist: release.artist,
  });
  return response.data.identification;
}

/** Administrator: forgets a manual match; automatic matching applies again. */
export async function clearAlbumIdentification(albumId: string) {
  await api.delete(`/metadata/identify/${segment(albumId)}`);
}
