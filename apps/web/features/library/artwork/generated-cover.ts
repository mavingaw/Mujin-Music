/**
 * Tasteful fallback artwork for albums without a cover: a dark field in one
 * of a few warm, low-saturation brand tones, an off-centre open ring (a nod
 * to the Ensō mark) and the title set small in the corner. Deterministic per
 * title so an album always gets the same art.
 */

const tones: readonly [string, string, string][] = [
  // [field top, field bottom, ring]
  ["#2a1d16", "#0d0a09", "#d98b5f"], // copper
  ["#1c2329", "#0a0c0e", "#8fc3e0"], // steel
  ["#1f251b", "#0a0c09", "#a3c98b"], // sage
  ["#28221a", "#0d0b09", "#daab6c"], // brass
  ["#221c20", "#0c0a0b", "#c9a196"], // clay
  ["#1d1d21", "#09090a", "#b8b3aa"], // graphite
];

/** FNV-1a over the title. */
export function coverSeed(text: string): number {
  let hash = 0x811c9dc5;
  for (const char of text) {
    hash ^= char.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash >>> 0;
}

function escapeXml(text: string): string {
  return text.replace(
    /[<>&'"]/g,
    (char) =>
      ({
        "<": "&lt;",
        ">": "&gt;",
        "&": "&amp;",
        "'": "&apos;",
        '"': "&quot;",
      })[char] ?? char,
  );
}

export function generatedCoverSvg(title: string, subtitle = ""): string {
  const seed = coverSeed(`${title}\u0000${subtitle}`);
  const [top, bottom, ring] = tones[seed % tones.length]!;
  const cx = 300 + ((seed >>> 4) % 120);
  const cy = 210 + ((seed >>> 11) % 110);
  const r = 120 + ((seed >>> 17) % 60);
  const gap = 0.6 + ((seed >>> 23) % 30) / 100;
  const start = ((seed >>> 7) % 360) * (Math.PI / 180);
  const end = start + Math.PI * 2 - gap;
  const x1 = cx + r * Math.cos(start);
  const y1 = cy + r * Math.sin(start);
  const x2 = cx + r * Math.cos(end);
  const y2 = cy + r * Math.sin(end);
  const label = escapeXml(title.trim().slice(0, 42) || "Untitled");
  const by = escapeXml(subtitle.trim().slice(0, 42));
  return `<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512">
<defs><linearGradient id="g" x1="0" y1="0" x2="0.4" y2="1"><stop offset="0" stop-color="${top}"/><stop offset="1" stop-color="${bottom}"/></linearGradient>
<radialGradient id="h" cx="${cx / 512}" cy="${cy / 512}" r="0.6"><stop offset="0" stop-color="${ring}" stop-opacity="0.16"/><stop offset="1" stop-color="${ring}" stop-opacity="0"/></radialGradient></defs>
<rect width="512" height="512" fill="url(#g)"/><rect width="512" height="512" fill="url(#h)"/>
<path d="M${x1.toFixed(1)} ${y1.toFixed(1)}A${r} ${r} 0 1 1 ${x2.toFixed(1)} ${y2.toFixed(1)}" fill="none" stroke="${ring}" stroke-opacity="0.55" stroke-width="${10 + (seed % 14)}"/>
<circle cx="${(cx + r * Math.cos(end + gap / 2)).toFixed(1)}" cy="${(cy + r * Math.sin(end + gap / 2)).toFixed(1)}" r="9" fill="${ring}"/>
<text x="36" y="452" fill="#f2eee6" fill-opacity="0.92" font-family="Jost, Inter, sans-serif" font-size="30" font-weight="400">${label}</text>
${by ? `<text x="36" y="484" fill="#b8b3aa" font-family="Inter, sans-serif" font-size="18">${by}</text>` : ""}
</svg>`;
}

export function generatedCover(title: string, subtitle = ""): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(generatedCoverSvg(title, subtitle))}`;
}
