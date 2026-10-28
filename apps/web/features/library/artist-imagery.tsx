"use client";

import { personInitials } from "@parson/music-sdk";
import Image from "next/image";
import { editorialImageSrc } from "@/features/library/credits/person-avatar";
import { generatedCover } from "@/features/library/artwork/generated-cover";
import getBaseURL from "@/lib/api/server-url";
import { getLibraryImageUrl } from "@/lib/images/image-url";
import { cn } from "@/lib/cn";

/**
 * A round artist picture: their photo (softly toned to the palette, full
 * colour on hover) when online metadata found one. Without a photo it is
 * honest about it: the artist's initials over a dimmed blur of one of their
 * covers, never a cover passed off as a portrait.
 */
export function ArtistRoundel({
  name,
  photoUrl,
  coverUrl,
  sizes = "180px",
  className,
}: {
  name: string;
  /** API path or URL of the artist's photo. */
  photoUrl?: string | null;
  /** Library path or URL of one of their covers, for the fallback. */
  coverUrl?: string | null;
  sizes?: string;
  className?: string;
}) {
  const photo = editorialImageSrc(photoUrl);
  const cover = getLibraryImageUrl(coverUrl, getBaseURL);
  return (
    <span
      className={cn(
        "relative mx-auto block aspect-square w-full overflow-hidden rounded-full bg-mujin-surface-1 shadow-[0_0_0_1px_rgb(var(--mj-ink)/0.07),0_18px_40px_-20px_rgba(0,0,0,0.9)]",
        photo && "mj-duotone mj-duotone-soft",
        className,
      )}
      data-artist-photo={photo ? "photo" : "monogram"}
    >
      {photo ? (
        <Image
          alt=""
          className="object-cover object-[50%_30%] transition-transform duration-500 ease-out motion-safe:group-hover:scale-[1.04]"
          fill
          sizes={sizes}
          src={photo}
        />
      ) : (
        <>
          {cover && (
            <Image
              alt=""
              aria-hidden="true"
              className="scale-125 object-cover opacity-45 blur-xl saturate-[0.7]"
              fill
              sizes="96px"
              src={cover}
            />
          )}
          <span
            aria-hidden="true"
            className="absolute inset-0 bg-[radial-gradient(80%_80%_at_30%_20%,rgb(var(--mj-accent-rgb)/0.22),transparent_70%),linear-gradient(to_bottom,rgb(var(--mj-canvas-rgb)/0.35),rgb(var(--mj-canvas-rgb)/0.7))]"
          />
          <span
            aria-hidden="true"
            className="mj-display absolute inset-0 grid place-items-center text-[clamp(22px,30%,56px)] text-mujin-text/90"
          >
            {personInitials(name)}
          </span>
        </>
      )}
    </span>
  );
}

/**
 * The picture behind a genre's name: photos of its artists side by side,
 * duotone in the brand's copper; without photos, a duotone collage of its
 * covers.
 */
export function GenreArt({
  name,
  photos = [],
  covers = [],
  sizes = "240px",
}: {
  name: string;
  photos?: string[];
  covers?: string[];
  sizes?: string;
}) {
  const photoSources = photos
    .map((path) => editorialImageSrc(path))
    .filter((src): src is string => Boolean(src))
    .slice(0, 3);
  if (photoSources.length) {
    return (
      <span
        aria-hidden="true"
        className="mj-duotone absolute inset-0 flex"
        data-genre-art="photos"
      >
        {photoSources.map((src) => (
          <span className="relative h-full flex-1" key={src}>
            <Image
              alt=""
              className="object-cover object-[50%_25%] transition-transform duration-700 ease-out motion-safe:group-hover:scale-[1.04]"
              fill
              sizes={sizes}
              src={src}
            />
          </span>
        ))}
      </span>
    );
  }
  const coverSources = (covers.length ? covers : [""])
    .map(
      (cover) =>
        getLibraryImageUrl(cover, getBaseURL) ?? generatedCover(name, ""),
    )
    .slice(0, 4);
  const grid = coverSources.length >= 4;
  return (
    <span
      aria-hidden="true"
      className={cn(
        "mj-duotone absolute inset-0 grid",
        grid ? "grid-cols-2 grid-rows-2" : "grid-cols-1",
      )}
      data-genre-art="covers"
    >
      {(grid ? coverSources : coverSources.slice(0, 1)).map((src, index) => (
        <span className="relative" key={`${src}-${index}`}>
          <Image
            alt=""
            className="object-cover transition-transform duration-500 ease-out motion-safe:group-hover:scale-[1.04]"
            fill
            sizes={sizes}
            src={src}
          />
        </span>
      ))}
    </span>
  );
}
