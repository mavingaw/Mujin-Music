/**
 * Appearance saves announced on the zone event socket. The zones provider
 * owns the socket and publishes here; the appearance provider listens and
 * re-reads the profile's appearance, so a theme chosen in one tab, window
 * or device restyles every other open session of the profile at once.
 */

type Listener = (revision: number | null) => void;

const listeners = new Set<Listener>();

export function publishAppearanceChange(revision?: number | null): void {
  const value =
    typeof revision === "number" && Number.isFinite(revision) ? revision : null;
  for (const listener of [...listeners]) listener(value);
}

export function subscribeAppearanceChanges(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * Whether an announced save needs a fetch: not when this session already
 * shows that revision (its own save) or has unsaved edits of its own (they
 * are saved on top of the newer revision, see the provider's flush).
 */
export function shouldFetchAnnounced(
  announced: number | null,
  shown: number,
  savingLocally: boolean,
): boolean {
  if (savingLocally) return false;
  return announced === null || announced !== shown;
}
