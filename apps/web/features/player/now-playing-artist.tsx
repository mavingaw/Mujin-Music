"use client";

import type { ArtistAbout } from "@parson/music-sdk/types";
import { editorialSourceName } from "@parson/music-sdk";
import Link from "next/link";
import { paragraphs } from "@/features/library/credits/credit-roles";
import { PersonAvatar } from "@/features/library/credits/person-avatar";

/** The first paragraph of a biography, cut at a sentence near `limit`. */
export function bioExcerpt(body: string, limit = 260): string {
  const first = paragraphs(body)[0] ?? body;
  const text = first.replace(/\s+/g, " ").trim();
  if (text.length <= limit) return text;
  const cut = text.slice(0, limit);
  const sentence = cut.lastIndexOf(". ");
  if (sentence > limit * 0.5) return cut.slice(0, sentence + 1);
  const word = cut.lastIndexOf(" ");
  return `${cut.slice(0, word > 0 ? word : limit).replace(/[,;:]$/, "")}…`;
}

/**
 * A short "About the artist" card for full-screen Now Playing: their photo,
 * the opening of their Wikipedia biography (with the attribution CC BY-SA
 * asks for) and a link to their page. Nothing renders without a biography.
 */
export function NowPlayingArtistBio({
  about,
  onNavigate,
}: {
  about: ArtistAbout | null | undefined;
  onNavigate: () => void;
}) {
  const biography = about?.biography;
  if (!about || !biography) return null;
  const source = editorialSourceName(
    `${biography.source}:${biography.language}`,
  );
  return (
    <section
      aria-label={`About ${about.name}`}
      className="mt-6 max-w-md rounded-lg border border-white/[0.07] bg-black/25 p-3.5 backdrop-blur-md md:[@media(max-height:820px)]:hidden"
    >
      <div className="flex items-center gap-3">
        <PersonAvatar imageUrl={about.image?.url} name={about.name} size={40} />
        <div className="min-w-0">
          <p className="mj-eyebrow text-[0.625rem]!">About the artist</p>
          <Link
            className="block truncate text-[0.875rem] font-medium text-mujin-text hover:underline"
            href={`/artist?id=${encodeURIComponent(about.artist_id)}`}
            onClick={onNavigate}
          >
            {about.name}
          </Link>
        </div>
      </div>
      <p
        className="mt-2.5 line-clamp-3 text-[0.8125rem] leading-[1.6] text-mujin-text-secondary"
        lang={biography.language}
      >
        {bioExcerpt(biography.body)}
      </p>
      <p className="mt-2 text-[0.6875rem] text-mujin-text-tertiary">
        From{" "}
        <a
          className="underline-offset-2 hover:text-mujin-text hover:underline"
          href={biography.url}
          rel="noreferrer"
          target="_blank"
        >
          {source}
        </a>{" "}
        ·{" "}
        <a
          className="underline-offset-2 hover:text-mujin-text hover:underline"
          href={biography.license_url}
          rel="noreferrer license"
          target="_blank"
        >
          {biography.license}
        </a>
      </p>
    </section>
  );
}
