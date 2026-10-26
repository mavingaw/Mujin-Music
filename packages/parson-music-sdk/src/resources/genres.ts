import api from "../core/http";
import type {
  Album,
  AlbumGenres,
  Artist,
  GenreDetails,
  GenreTree,
} from "../domain/types";

export interface GenrePage {
  limit?: number;
  offset?: number;
}

export async function listAllGenres(): Promise<string[]> {
  const response = await api.get("/genres");
  return response.data;
}

export async function getAlbumsByGenres(
  genres: string[],
  page: GenrePage = {},
): Promise<Album[]> {
  const genresQuery = genres.join("+");
  const response = await api.get(
    `/genres/${encodeURIComponent(genresQuery)}/albums`,
    {
      params: page,
    },
  );
  return response.data;
}

export async function getArtistsByGenres(
  genres: string[],
  page: GenrePage = {},
): Promise<Artist[]> {
  const genresQuery = genres.join("+");
  const response = await api.get(
    `/genres/${encodeURIComponent(genresQuery)}/artists`,
    {
      params: page,
    },
  );
  return response.data;
}

interface LibrarySong {
  id: string;
  name: string;
  artist: string;
  contributing_artists: string[];
  track_number: number;
  path: string;
  duration: number;
}

export async function getSongsByGenres(
  genres: string[],
  page: GenrePage = {},
): Promise<LibrarySong[]> {
  const genresQuery = genres.join("+");
  const response = await api.get(
    `/genres/${encodeURIComponent(genresQuery)}/songs`,
    {
      params: page,
    },
  );
  return response.data;
}

export interface PopularGenre {
  name: string;
  song_count: number;
  cover_image?: string | null;
}

export async function getPopularGenres(): Promise<PopularGenre[]> {
  const res = await api.get(`/genres/popular`);
  return res.data as PopularGenre[];
}

const segment = (value: string) => encodeURIComponent(value);

/** The library's genres as a canonical tree with album counts. */
export async function getGenreTree(): Promise<GenreTree> {
  const response = await api.get<GenreTree>("/genres/canonical");
  return response.data;
}

/** A genre page: About text, sub-genres, albums, artists and composers. */
export async function getGenrePage(slug: string): Promise<GenreDetails> {
  const response = await api.get<GenreDetails>(
    `/genres/canonical/${segment(slug)}`,
  );
  return response.data;
}

/** Administrator: chooses an album's genres by hand. */
export async function setAlbumGenres(
  albumId: string,
  genres: string[],
): Promise<AlbumGenres> {
  const response = await api.put<AlbumGenres>(
    `/metadata/albums/${segment(albumId)}/genres`,
    { genres },
  );
  return response.data;
}

/** Administrator: goes back to tag and online genres for an album. */
export async function resetAlbumGenres(albumId: string): Promise<AlbumGenres> {
  const response = await api.delete<AlbumGenres>(
    `/metadata/albums/${segment(albumId)}/genres`,
  );
  return response.data;
}
