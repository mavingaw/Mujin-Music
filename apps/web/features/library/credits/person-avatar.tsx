"use client";

import { getApiBaseURL, personHue, personInitials } from "@parson/music-sdk";
import Image from "next/image";
import { useState } from "react";

/** Turns a server path such as `/artists/{id}/image` into a full URL. */
export function editorialImageSrc(path: string | null | undefined) {
  if (!path) return null;
  return /^https?:\/\//.test(path) ? path : `${getApiBaseURL()}${path}`;
}

/**
 * A round photo of a person, or their initials on a color of their own when
 * there is no photo (or it fails to load).
 */
export function PersonAvatar({
  name,
  imageUrl,
  size = 36,
  className = "",
  priority = false,
}: {
  name: string;
  /** Full URL or server path of the photo. */
  imageUrl?: string | null;
  size?: number;
  className?: string;
  priority?: boolean;
}) {
  const src = editorialImageSrc(imageUrl);
  const [failed, setFailed] = useState<string | null>(null);
  const showImage = src && failed !== src;
  const hue = personHue(name);
  return (
    <span
      aria-hidden={showImage ? undefined : true}
      className={`relative inline-flex shrink-0 items-center justify-center overflow-hidden rounded-full font-semibold text-white/90 ring-1 ring-white/10 ${className}`}
      style={{
        width: size,
        height: size,
        fontSize: Math.max(10, Math.round(size * 0.36)),
        background: showImage
          ? "rgb(39 39 42)"
          : `linear-gradient(135deg, hsl(${hue} 32% 34%), hsl(${(hue + 40) % 360} 38% 20%))`,
      }}
    >
      {showImage ? (
        <Image
          alt={name}
          className="object-cover"
          fill
          onError={() => setFailed(src)}
          priority={priority}
          sizes={`${size}px`}
          src={src}
        />
      ) : (
        <span className="select-none tracking-wide">
          {personInitials(name)}
        </span>
      )}
    </span>
  );
}
