import api from "../core/http";
import type {
  AlbumField,
  AlbumSourcesPatch,
  EditedCredit,
  FactSource,
  FieldSources,
} from "../domain/types";

const segment = (id: string) => encodeURIComponent(id);

/**
 * Administrator: chooses Tags or Online per field for one album (`null`
 * follows the library default) and freezes typed values (`null` unfreezes).
 */
export async function updateAlbumSources(
  albumId: string,
  patch: AlbumSourcesPatch,
): Promise<void> {
  await api.put(`/metadata/albums/${segment(albumId)}/sources`, {
    sources: patch.sources ?? {},
    edits: patch.edits ?? {},
  });
}

/**
 * Administrator: replaces an album's credits with an edited list. The list
 * is frozen: looking the album up again never changes it.
 */
export async function saveAlbumCredits(
  albumId: string,
  credits: EditedCredit[],
): Promise<EditedCredit[]> {
  const response = await api.put<{ credits: EditedCredit[] }>(
    `/metadata/albums/${segment(albumId)}/credits`,
    { credits },
  );
  return response.data.credits;
}

/** Administrator: goes back to the credits in the tags and online. */
export async function revertAlbumCredits(albumId: string): Promise<void> {
  await api.delete(`/metadata/albums/${segment(albumId)}/credits`);
}

/** Administrator: the library-wide default source per field. */
export async function getLibrarySources(): Promise<FieldSources> {
  const response = await api.get<FieldSources>("/metadata/sources");
  return response.data;
}

/** Administrator: changes library-wide defaults; other fields keep theirs. */
export async function updateLibrarySources(
  changes: Partial<Record<AlbumField, FactSource>>,
): Promise<FieldSources> {
  const response = await api.put<FieldSources>("/metadata/sources", changes);
  return response.data;
}
