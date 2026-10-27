/**
 * A cover's colour swatch from the server (`/media/images/<cover>?format=
 * swatch`: 24×24 RGBA pixels, base64). Clients with no canvas (the phone
 * app) score it with the same `sourceColorFromPixels` the web app uses on
 * the full image, so both pick the same Material You source colour.
 */

import type { Argb } from "./color";
import { FALLBACK_SOURCE, sourceColorFromPixels } from "./artwork-color";

const ALPHABET =
  "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/** Decodes standard base64 without relying on `atob` (Hermes, workers). */
export function bytesFromBase64(text: string): Uint8Array {
  const clean = text.replace(/[^A-Za-z0-9+/]/g, "");
  const bytes = new Uint8Array(Math.floor((clean.length * 3) / 4));
  let buffer = 0;
  let bits = 0;
  let index = 0;
  for (const character of clean) {
    buffer = (buffer << 6) | ALPHABET.indexOf(character);
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      bytes[index++] = (buffer >> bits) & 255;
    }
  }
  return bytes.subarray(0, index);
}

export interface CoverSwatch {
  width: number;
  height: number;
  rgba: string;
}

export function sourceColorFromSwatch(
  swatch: CoverSwatch | null | undefined,
): Argb {
  if (!swatch?.rgba) return FALLBACK_SOURCE;
  return sourceColorFromPixels(bytesFromBase64(swatch.rgba));
}
