/**
 * HCT (hue, chroma, tone): the colour space behind Material 3's dynamic
 * colour. Hue and chroma come from CAM16 under default viewing conditions;
 * tone is CIE L*, so a tone difference is a contrast difference. This is an
 * independent implementation of the published algorithm (the same model as
 * Google's Material colour utilities, Apache-2.0), with the solver that
 * finds the most chromatic in-gamut sRGB colour for a requested HCT.
 */

import {
  type Argb,
  argbFromLstar,
  argbFromXyz,
  lstarFromArgb,
  lstarFromY,
  sanitizeDegrees,
  signum,
  WHITE_POINT_D65,
  xyzFromArgb,
  yFromLstar,
} from "./color";

const XYZ_TO_CAM16RGB = [
  [0.401288, 0.650173, -0.051461],
  [-0.250268, 1.204414, 0.045854],
  [-0.002079, 0.048952, 0.953127],
];

const CAM16RGB_TO_XYZ = [
  [1.8620678, -1.0112547, 0.14918678],
  [0.38752654, 0.62144744, -0.00897398],
  [-0.0158415, -0.03412294, 1.0499644],
];

interface ViewingConditions {
  n: number;
  aw: number;
  nbb: number;
  ncb: number;
  c: number;
  nc: number;
  rgbD: [number, number, number];
  fl: number;
  flRoot: number;
  z: number;
}

function lerp(start: number, stop: number, amount: number) {
  return (1 - amount) * start + amount * stop;
}

function makeViewingConditions(): ViewingConditions {
  const whitePoint = WHITE_POINT_D65;
  const adaptingLuminance = ((200 / Math.PI) * yFromLstar(50)) / 100;
  const backgroundLstar = 50;
  const surround = 2;
  const [rW, gW, bW] = XYZ_TO_CAM16RGB.map(
    (row) =>
      whitePoint[0]! * row[0]! +
      whitePoint[1]! * row[1]! +
      whitePoint[2]! * row[2]!,
  ) as [number, number, number];
  const f = 0.8 + surround / 10;
  const c =
    f >= 0.9
      ? lerp(0.59, 0.69, (f - 0.9) * 10)
      : lerp(0.525, 0.59, (f - 0.8) * 10);
  let d = f * (1 - (1 / 3.6) * Math.exp((-adaptingLuminance - 42) / 92));
  d = Math.max(0, Math.min(1, d));
  const nc = f;
  const rgbD: [number, number, number] = [
    d * (100 / rW) + 1 - d,
    d * (100 / gW) + 1 - d,
    d * (100 / bW) + 1 - d,
  ];
  const k = 1 / (5 * adaptingLuminance + 1);
  const k4 = k * k * k * k;
  const k4F = 1 - k4;
  const fl =
    k4 * adaptingLuminance + 0.1 * k4F * k4F * Math.cbrt(5 * adaptingLuminance);
  const n = yFromLstar(backgroundLstar) / whitePoint[1]!;
  const z = 1.48 + Math.sqrt(n);
  const nbb = 0.725 / Math.pow(n, 0.2);
  const ncb = nbb;
  const rgbAFactors = [
    Math.pow((fl * rgbD[0] * rW) / 100, 0.42),
    Math.pow((fl * rgbD[1] * gW) / 100, 0.42),
    Math.pow((fl * rgbD[2] * bW) / 100, 0.42),
  ];
  const rgbA = rgbAFactors.map((factor) => (400 * factor) / (factor + 27.13));
  const aw = (2 * rgbA[0]! + rgbA[1]! + 0.05 * rgbA[2]!) * nbb;
  return { n, aw, nbb, ncb, c, nc, rgbD, fl, flRoot: Math.pow(fl, 0.25), z };
}

const VC = makeViewingConditions();

/** A colour in CAM16 (default viewing conditions). */
export interface Cam16 {
  hue: number;
  chroma: number;
  j: number;
  jstar: number;
  astar: number;
  bstar: number;
}

function ucs(j: number, m: number, hue: number) {
  const hueRadians = (hue * Math.PI) / 180;
  const jstar = ((1 + 100 * 0.007) * j) / (1 + 0.007 * j);
  const mstar = (1 / 0.0228) * Math.log1p(0.0228 * m);
  return {
    jstar,
    astar: mstar * Math.cos(hueRadians),
    bstar: mstar * Math.sin(hueRadians),
  };
}

export function cam16FromArgb(argb: Argb): Cam16 {
  const [x, y, z] = xyzFromArgb(argb);
  const rC =
    XYZ_TO_CAM16RGB[0]![0]! * x +
    XYZ_TO_CAM16RGB[0]![1]! * y +
    XYZ_TO_CAM16RGB[0]![2]! * z;
  const gC =
    XYZ_TO_CAM16RGB[1]![0]! * x +
    XYZ_TO_CAM16RGB[1]![1]! * y +
    XYZ_TO_CAM16RGB[1]![2]! * z;
  const bC =
    XYZ_TO_CAM16RGB[2]![0]! * x +
    XYZ_TO_CAM16RGB[2]![1]! * y +
    XYZ_TO_CAM16RGB[2]![2]! * z;
  const adapt = (component: number, index: 0 | 1 | 2) => {
    const d = VC.rgbD[index] * component;
    const af = Math.pow((VC.fl * Math.abs(d)) / 100, 0.42);
    return (signum(d) * 400 * af) / (af + 27.13);
  };
  const rA = adapt(rC, 0);
  const gA = adapt(gC, 1);
  const bA = adapt(bC, 2);
  const a = (11 * rA + -12 * gA + bA) / 11;
  const b = (rA + gA - 2 * bA) / 9;
  const u = (20 * rA + 20 * gA + 21 * bA) / 20;
  const p2 = (40 * rA + 20 * gA + bA) / 20;
  const hue = sanitizeDegrees((Math.atan2(b, a) * 180) / Math.PI);
  const ac = p2 * VC.nbb;
  const j = 100 * Math.pow(ac / VC.aw, VC.c * VC.z);
  const huePrime = hue < 20.14 ? hue + 360 : hue;
  const eHue = 0.25 * (Math.cos((huePrime * Math.PI) / 180 + 2) + 3.8);
  const p1 = (50000 / 13) * eHue * VC.nc * VC.ncb;
  const t = (p1 * Math.hypot(a, b)) / (u + 0.305);
  const alpha = Math.pow(t, 0.9) * Math.pow(1.64 - Math.pow(0.29, VC.n), 0.73);
  const chroma = alpha * Math.sqrt(j / 100);
  const m = chroma * VC.flRoot;
  return { hue, chroma, j, ...ucs(j, m, hue) };
}

export function cam16FromJch(j: number, chroma: number, hue: number): Cam16 {
  const m = chroma * VC.flRoot;
  return { hue, chroma, j, ...ucs(j, m, hue) };
}

/** CAM16-UCS distance. */
export function cam16Distance(a: Cam16, b: Cam16): number {
  const dJ = a.jstar - b.jstar;
  const dA = a.astar - b.astar;
  const dB = a.bstar - b.bstar;
  return 1.41 * Math.pow(Math.sqrt(dJ * dJ + dA * dA + dB * dB), 0.63);
}

/** The sRGB colour (clipped) a CAM16 colour shows as. */
export function argbFromCam16(cam: Cam16): Argb {
  const alpha =
    cam.chroma === 0 || cam.j === 0 ? 0 : cam.chroma / Math.sqrt(cam.j / 100);
  const t = Math.pow(
    alpha / Math.pow(1.64 - Math.pow(0.29, VC.n), 0.73),
    1 / 0.9,
  );
  const hRad = (cam.hue * Math.PI) / 180;
  const eHue = 0.25 * (Math.cos(hRad + 2) + 3.8);
  const ac = VC.aw * Math.pow(cam.j / 100, 1 / VC.c / VC.z);
  const p1 = eHue * (50000 / 13) * VC.nc * VC.ncb;
  const p2 = ac / VC.nbb;
  const hSin = Math.sin(hRad);
  const hCos = Math.cos(hRad);
  const gamma =
    (23 * (p2 + 0.305) * t) / (23 * p1 + 11 * t * hCos + 108 * t * hSin);
  const a = gamma * hCos;
  const b = gamma * hSin;
  const rA = (460 * p2 + 451 * a + 288 * b) / 1403;
  const gA = (460 * p2 - 891 * a - 261 * b) / 1403;
  const bA = (460 * p2 - 220 * a - 6300 * b) / 1403;
  const unadapt = (component: number, index: 0 | 1 | 2) => {
    const base = Math.max(
      0,
      (27.13 * Math.abs(component)) / (400 - Math.abs(component)),
    );
    return (
      (signum(component) * (100 / VC.fl) * Math.pow(base, 1 / 0.42)) /
      VC.rgbD[index]
    );
  };
  const rF = unadapt(rA, 0);
  const gF = unadapt(gA, 1);
  const bF = unadapt(bA, 2);
  const [x, y, z] = CAM16RGB_TO_XYZ.map(
    (row) => row[0]! * rF + row[1]! * gF + row[2]! * bF,
  ) as [number, number, number];
  return argbFromXyz(x, y, z);
}

/** The most fitting CAM16 colour with this hue and chroma at tone `lstar`. */
function findCamByJ(hue: number, chroma: number, lstar: number): Cam16 | null {
  let low = 0;
  let high = 100;
  let bestDeltaL = 1000;
  let bestDeltaE = 1000;
  let best: Cam16 | null = null;
  while (Math.abs(low - high) > 0.01) {
    const mid = low + (high - low) / 2;
    const clipped = argbFromCam16(cam16FromJch(mid, chroma, hue));
    const clippedLstar = lstarFromArgb(clipped);
    const deltaL = Math.abs(lstar - clippedLstar);
    if (deltaL < 0.2) {
      const camClipped = cam16FromArgb(clipped);
      const deltaE = cam16Distance(
        camClipped,
        cam16FromJch(camClipped.j, camClipped.chroma, hue),
      );
      if (deltaE <= 1 && deltaE <= bestDeltaE) {
        bestDeltaL = deltaL;
        bestDeltaE = deltaE;
        best = camClipped;
      }
    }
    if (bestDeltaL === 0 && bestDeltaE === 0) break;
    if (clippedLstar < lstar) low = mid;
    else high = mid;
  }
  return best;
}

/** The sRGB colour for hue, chroma and tone, reducing chroma to fit sRGB. */
export function argbFromHct(hue: number, chroma: number, tone: number): Argb {
  if (chroma < 1 || Math.round(tone) <= 0 || Math.round(tone) >= 100) {
    return argbFromLstar(tone);
  }
  const h = sanitizeDegrees(hue);
  let high = chroma;
  let mid = chroma;
  let low = 0;
  let firstLoop = true;
  let answer: Cam16 | null = null;
  while (Math.abs(low - high) >= 0.4) {
    const possible = findCamByJ(h, mid, tone);
    if (firstLoop) {
      if (possible) return argbFromCam16(possible);
      firstLoop = false;
      mid = low + (high - low) / 2;
      continue;
    }
    if (possible) {
      answer = possible;
      low = mid;
    } else {
      high = mid;
    }
    mid = low + (high - low) / 2;
  }
  return answer ? argbFromCam16(answer) : argbFromLstar(tone);
}

export interface Hct {
  hue: number;
  chroma: number;
  tone: number;
}

export function hctFromArgb(argb: Argb): Hct {
  const cam = cam16FromArgb(argb);
  return { hue: cam.hue, chroma: cam.chroma, tone: lstarFromArgb(argb) };
}

/** L* of a Y value, re-exported for palette code. */
export { lstarFromY };
