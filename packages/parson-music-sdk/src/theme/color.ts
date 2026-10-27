/**
 * Colour arithmetic for the theme engine: sRGB <-> linear <-> CIE XYZ and
 * L*, WCAG 2.x relative luminance and contrast. Colours travel as ARGB
 * integers (0xAARRGGBB, like Material's colour utilities) or `#rrggbb`.
 */

export type Argb = number;

const SRGB_TO_XYZ = [
  [0.41233895, 0.35762064, 0.18051042],
  [0.2126, 0.7152, 0.0722],
  [0.01932141, 0.11916382, 0.95034478],
];

const XYZ_TO_SRGB = [
  [3.2413774792388685, -1.5376652402851851, -0.49885366846268053],
  [-0.9691452513005321, 1.8758853451067872, 0.04156585323298229],
  [0.05562093689691305, -0.20395524564742123, 1.0571799111220335],
];

/** D65, Y = 100. */
export const WHITE_POINT_D65 = [95.047, 100.0, 108.883];

export function argbFromRgb(red: number, green: number, blue: number): Argb {
  return (
    ((255 << 24) |
      ((red & 255) << 16) |
      ((green & 255) << 8) |
      (blue & 255)) >>>
    0
  );
}

export const redFromArgb = (argb: Argb) => (argb >> 16) & 255;
export const greenFromArgb = (argb: Argb) => (argb >> 8) & 255;
export const blueFromArgb = (argb: Argb) => argb & 255;

/** `#rgb`, `#rrggbb` or `#aarrggbb`-free input; null when it is not a colour. */
export function argbFromHex(hex: string): Argb | null {
  const value = hex.trim().replace(/^#/, "");
  if (/^[0-9a-f]{3}$/i.test(value)) {
    const [r, g, b] = value
      .split("")
      .map((digit) => parseInt(digit + digit, 16));
    return argbFromRgb(r!, g!, b!);
  }
  if (/^[0-9a-f]{6}$/i.test(value)) {
    return argbFromRgb(
      parseInt(value.slice(0, 2), 16),
      parseInt(value.slice(2, 4), 16),
      parseInt(value.slice(4, 6), 16),
    );
  }
  return null;
}

export function hexFromArgb(argb: Argb): string {
  return `#${[redFromArgb(argb), greenFromArgb(argb), blueFromArgb(argb)]
    .map((part) => part.toString(16).padStart(2, "0"))
    .join("")}`;
}

/** "217 139 95" for `rgb(var(--x) / 0.5)`-style use. */
export function rgbTriplet(argb: Argb): string {
  return `${redFromArgb(argb)} ${greenFromArgb(argb)} ${blueFromArgb(argb)}`;
}

export function rgba(argb: Argb, alpha: number): string {
  return `rgba(${redFromArgb(argb)},${greenFromArgb(argb)},${blueFromArgb(argb)},${Math.round(alpha * 1000) / 1000})`;
}

/** 0..255 component -> 0..100 linear. */
export function linearized(component: number): number {
  const normalized = component / 255;
  return normalized <= 0.040449936
    ? (normalized / 12.92) * 100
    : Math.pow((normalized + 0.055) / 1.055, 2.4) * 100;
}

/** 0..100 linear -> 0..255 component, clamped and rounded. */
export function delinearized(component: number): number {
  const normalized = component / 100;
  const value =
    normalized <= 0.0031308
      ? normalized * 12.92
      : 1.055 * Math.pow(normalized, 1 / 2.4) - 0.055;
  return Math.max(0, Math.min(255, Math.round(value * 255)));
}

export function xyzFromArgb(argb: Argb): [number, number, number] {
  const r = linearized(redFromArgb(argb));
  const g = linearized(greenFromArgb(argb));
  const b = linearized(blueFromArgb(argb));
  return SRGB_TO_XYZ.map((row) => row[0]! * r + row[1]! * g + row[2]! * b) as [
    number,
    number,
    number,
  ];
}

export function argbFromXyz(x: number, y: number, z: number): Argb {
  const [r, g, b] = XYZ_TO_SRGB.map(
    (row) => row[0]! * x + row[1]! * y + row[2]! * z,
  ) as [number, number, number];
  return argbFromRgb(delinearized(r), delinearized(g), delinearized(b));
}

/** Whether linear RGB (0..100) is inside the sRGB cube. */
export function xyzInGamut(x: number, y: number, z: number): boolean {
  return XYZ_TO_SRGB.every((row) => {
    const value = row[0]! * x + row[1]! * y + row[2]! * z;
    return value >= -0.01 && value <= 100.01;
  });
}

const EPSILON = 216 / 24389;
const KAPPA = 24389 / 27;

function labF(t: number): number {
  return t > EPSILON ? Math.cbrt(t) : (KAPPA * t + 16) / 116;
}

function labInvf(ft: number): number {
  const ft3 = ft * ft * ft;
  return ft3 > EPSILON ? ft3 : (116 * ft - 16) / KAPPA;
}

/** CIE L* (0..100) from Y (0..100). */
export function lstarFromY(y: number): number {
  return 116 * labF(y / 100) - 16;
}

/** Y (0..100) from CIE L*. */
export function yFromLstar(lstar: number): number {
  return 100 * labInvf((lstar + 16) / 116);
}

export function lstarFromArgb(argb: Argb): number {
  return lstarFromY(xyzFromArgb(argb)[1]);
}

/** The grey with this L*. */
export function argbFromLstar(lstar: number): Argb {
  const component = delinearized(yFromLstar(lstar));
  return argbFromRgb(component, component, component);
}

/** WCAG 2.x relative luminance, 0..1. */
export function relativeLuminance(argb: Argb): number {
  return xyzFromArgb(argb)[1] / 100;
}

/** WCAG 2.x contrast ratio between two colours (1..21). */
export function contrastRatio(a: Argb, b: Argb): number {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  const [light, dark] = la > lb ? [la, lb] : [lb, la];
  return (light + 0.05) / (dark + 0.05);
}

/** Straight-alpha blend of `top` over `bottom`. */
export function blend(bottom: Argb, top: Argb, alpha: number): Argb {
  const mix = (a: number, b: number) => Math.round(a + (b - a) * alpha);
  return argbFromRgb(
    mix(redFromArgb(bottom), redFromArgb(top)),
    mix(greenFromArgb(bottom), greenFromArgb(top)),
    mix(blueFromArgb(bottom), blueFromArgb(top)),
  );
}

/** Interprets a matrix row product; exported for the CAM16 code. */
export function matrixMultiply(
  row: readonly number[],
  matrix: readonly (readonly number[])[],
): [number, number, number] {
  return matrix.map(
    (line) => line[0]! * row[0]! + line[1]! * row[1]! + line[2]! * row[2]!,
  ) as [number, number, number];
}

export function sanitizeDegrees(degrees: number): number {
  const value = degrees % 360;
  return value < 0 ? value + 360 : value;
}

export function signum(value: number): number {
  return value < 0 ? -1 : value === 0 ? 0 : 1;
}
