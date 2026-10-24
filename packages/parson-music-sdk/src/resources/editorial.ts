import api from "../core/http";
import type {
  AlbumAbout,
  ArtistAbout,
  ArtistCredits,
  EnrichmentOverview,
  EnrichmentSettings,
  UserNote,
} from "../domain/types";

const segment = (id: string) => encodeURIComponent(id);

/** Credits, background, review, release facts and your note for an album. */
export async function getAlbumAbout(id: string): Promise<AlbumAbout> {
  const response = await api.get<AlbumAbout>(`/albums/${segment(id)}/about`);
  return response.data;
}

/** Biography, licensed photo, facts, credit roles and your note for an artist. */
export async function getArtistAbout(id: string): Promise<ArtistAbout> {
  const response = await api.get<ArtistAbout>(`/artists/${segment(id)}/about`);
  return response.data;
}

/** Everything a person composed, produced, performed on, ... in the library. */
export async function getArtistCredits(id: string): Promise<ArtistCredits> {
  const response = await api.get<ArtistCredits>(
    `/artists/${segment(id)}/credits`,
  );
  return response.data;
}

/** API path of an artist's stored photo, relative to the API base URL. */
export function artistEditorialImagePath(id: string): string {
  return `/artists/${segment(id)}/image`;
}

/** Saves a private note; an empty body deletes it. */
export async function saveNote(
  entityType: "album" | "artist",
  id: string,
  body: string,
): Promise<UserNote | null> {
  const response = await api.put<{ note: UserNote | null }>(
    `/${entityType === "album" ? "albums" : "artists"}/${segment(id)}/note`,
    { body },
  );
  return response.data.note;
}

/** Administrator: enrichment settings and progress. */
export async function getEnrichment(): Promise<EnrichmentOverview> {
  const response = await api.get<EnrichmentOverview>("/metadata/enrichment");
  return response.data;
}

/** Administrator: turns enrichment on or off and changes its options. */
export async function updateEnrichment(
  patch: Partial<EnrichmentSettings>,
): Promise<EnrichmentOverview> {
  const response = await api.put<EnrichmentOverview>(
    "/metadata/enrichment",
    patch,
  );
  return response.data;
}

/** Administrator: looks an album or artist up again. */
export async function refreshEnrichment(
  entityType: "album" | "artist",
  entityId: string,
): Promise<void> {
  await api.post("/metadata/enrichment/refresh", {
    entity_type: entityType,
    entity_id: entityId,
  });
}
