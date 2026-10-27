/**
 * Phones float one bar above the tab bar: the mini player while something
 * plays, or the idle zone bar. `html[data-phone-dock]` tells the page to
 * reserve room for it (see globals.css), so the end of every page scrolls
 * clear of it instead of sitting underneath.
 *
 * Several components can hold the dock at once (a route change mounts the
 * next bar before the last one leaves), so the attribute is a count.
 */
export function nextDockCount(
  current: string | undefined,
  delta: 1 | -1,
): string | null {
  const count = Math.max(0, Number(current ?? "0") || 0) + delta;
  return count > 0 ? String(count) : null;
}

export function holdPhoneDock(root: { dataset: DOMStringMap }): () => void {
  const apply = (delta: 1 | -1) => {
    const next = nextDockCount(root.dataset.phoneDock, delta);
    if (next) root.dataset.phoneDock = next;
    else delete root.dataset.phoneDock;
  };
  apply(1);
  return () => apply(-1);
}
