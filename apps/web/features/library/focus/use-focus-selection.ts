"use client";

import {
  focusSelectionFromParams,
  focusSelectionToParams,
  type FocusSelection,
} from "@parson/music-sdk";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useCallback, useMemo } from "react";

/**
 * A Focus selection kept in the URL (`?conductor=…&bit_depth=24`), so a
 * focused discography can be shared, bookmarked and restored with Back.
 */
export function useFocusSelection(extra?: Record<string, string>) {
  const searchParams = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const key = searchParams?.toString() ?? "";
  const selection = useMemo(
    () => focusSelectionFromParams(new URLSearchParams(key)),
    [key],
  );
  const extraKey = JSON.stringify(extra ?? {});
  const setSelection = useCallback(
    (next: FocusSelection) => {
      const params = focusSelectionToParams(next, key);
      for (const [name, value] of Object.entries(
        JSON.parse(extraKey) as Record<string, string>,
      )) {
        params.set(name, value);
      }
      router.replace(`${pathname}?${params.toString()}`, { scroll: false });
    },
    [extraKey, key, pathname, router],
  );
  return [selection, setSelection] as const;
}
