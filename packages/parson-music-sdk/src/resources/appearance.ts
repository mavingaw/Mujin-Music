import api, { isApiError } from "../core/http";
import { type Appearance, normalizeAppearance } from "../theme/appearance";

/** A profile's saved appearance and its revision (0 = never saved). */
export interface AppearanceRecord {
  appearance: Appearance;
  revision: number;
  /** Unix seconds of the last save. */
  updated_at: number | null;
}

function record(data: unknown): AppearanceRecord {
  const value = (data ?? {}) as Partial<AppearanceRecord>;
  return {
    appearance: normalizeAppearance(value.appearance),
    revision: Number.isInteger(value.revision) ? Number(value.revision) : 0,
    updated_at: typeof value.updated_at === "number" ? value.updated_at : null,
  };
}

export async function getAppearance(): Promise<AppearanceRecord> {
  const response = await api.get("/users/me/appearance");
  return record(response.data);
}

/**
 * Saves the profile's appearance. With `baseRevision`, a save made after
 * another device's save is refused; read it with `appearanceConflict`.
 */
export async function saveAppearance(
  appearance: Appearance,
  baseRevision?: number,
): Promise<AppearanceRecord> {
  const response = await api.put("/users/me/appearance", {
    appearance,
    ...(baseRevision === undefined ? {} : { base_revision: baseRevision }),
  });
  return record(response.data);
}

/** The newer saved appearance a 409 carries, or null for other errors. */
export function appearanceConflict(error: unknown): AppearanceRecord | null {
  if (!isApiError(error) || error.response?.status !== 409) return null;
  const data = error.response.data as {
    error?: string;
    current?: unknown;
  } | null;
  return data?.error === "appearance_conflict" ? record(data.current) : null;
}
