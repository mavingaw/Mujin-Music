/** Route of a person's page; contributor ids such as `mb:<id>` are encoded. */
export function artistRoute(id: string, tab?: "credits") {
  const path = `/artist/${encodeURIComponent(id)}`;
  return tab ? `${path}?tab=${tab}` : path;
}

/** "1959 · Columbia · FLAC 96 kHz 24-bit", skipping missing parts. */
export function joinMeta(parts: readonly (string | null | undefined)[]) {
  return parts.filter((part): part is string => Boolean(part)).join(" · ");
}

/** Route of a work page. */
export function workRoute(mbid: string) {
  return `/work/${encodeURIComponent(mbid)}`;
}

/** Route of a genre page. */
export function genreRoute(slug: string) {
  return `/genre/${encodeURIComponent(slug)}`;
}
