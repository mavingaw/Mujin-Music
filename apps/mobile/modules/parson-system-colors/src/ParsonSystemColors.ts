import { requireOptionalNativeModule } from "expo";

/** Android 12+ system palette seeds as signed ARGB integers. */
export type SystemColors = {
  accent1: number;
  accent2: number;
  accent3: number;
  neutral1: number;
  neutral2: number;
};

type NativeSystemColors = {
  getSystemColors(): SystemColors | null;
};

const native =
  requireOptionalNativeModule<NativeSystemColors>("ParsonSystemColors");

/** The wallpaper palette, or null before Android 12, on iOS and on web. */
export function getSystemColors(): SystemColors | null {
  try {
    return native?.getSystemColors() ?? null;
  } catch {
    return null;
  }
}
