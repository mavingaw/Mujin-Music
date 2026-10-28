"use client";

import {
  type Appearance,
  type NativeMaterial,
  type ResolvedTheme,
  type ThemeEnvironment,
  appearanceConflict,
  DEFAULT_APPEARANCE,
  getAppearance,
  normalizeAppearance,
  resolveTheme,
  saveAppearance,
} from "@parson/music-sdk";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useSession } from "@/features/account/session-provider";
import {
  desktopWindowMaterials,
  setDesktopWindowMaterial,
} from "@/lib/desktop/bridge";
import {
  shouldFetchAnnounced,
  subscribeAppearanceChanges,
} from "./appearance-events";
import { applyThemeToDocument } from "./apply-theme";
import {
  buildThemeCache,
  readThemeCache,
  THEME_CACHE_KEY,
} from "./theme-cache";
import { useArtworkSourceColor } from "./use-artwork-source";

export type AppearanceSyncStatus = "local" | "saving" | "saved" | "error";

interface AppearanceContextValue {
  appearance: Appearance;
  theme: ResolvedTheme;
  environment: ThemeEnvironment;
  revision: number;
  status: AppearanceSyncStatus;
  /** Applies at once (live preview) and saves to the profile shortly after. */
  update: (patch: Partial<Appearance>) => void;
  reset: () => void;
}

const AppearanceContext = createContext<AppearanceContextValue | null>(null);

const SAVE_DELAY_MS = 450;
const SYNC_INTERVAL_MS = 30_000;

function storage(): Storage | null {
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    return null;
  }
}

function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(false);
  useEffect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const media = window.matchMedia(query);
    const sync = () => setMatches(media.matches);
    sync();
    media.addEventListener("change", sync);
    return () => media.removeEventListener("change", sync);
  }, [query]);
  return matches;
}

export function AppearanceProvider({
  artworkSrc,
  children,
}: {
  /** The playing album's cover, for colour from artwork. */
  artworkSrc?: string | null;
  children: ReactNode;
}) {
  const { session } = useSession();
  const userId = session?.sub ?? null;
  const [appearance, setAppearance] = useState<Appearance>(DEFAULT_APPEARANCE);
  const [revision, setRevision] = useState(0);
  const [status, setStatus] = useState<AppearanceSyncStatus>("local");
  const [nativeMaterials, setNativeMaterials] = useState<NativeMaterial[]>([]);
  const systemDark = useMediaQuery("(prefers-color-scheme: dark)");
  const reducedMotion = useMediaQuery("(prefers-reduced-motion: reduce)");
  const reducedTransparency = useMediaQuery(
    "(prefers-reduced-transparency: reduce)",
  );
  const [mediaReady, setMediaReady] = useState(false);
  const pending = useRef<Partial<Appearance> | null>(null);
  const saveTimer = useRef<number | null>(null);
  const revisionRef = useRef(0);
  const appearanceRef = useRef(appearance);
  appearanceRef.current = appearance;
  revisionRef.current = revision;

  // Start from what this browser last showed for this profile.
  useEffect(() => {
    const cached = readThemeCache(storage()?.getItem(THEME_CACHE_KEY) ?? null);
    if (cached && (!userId || !cached.userId || cached.userId === userId)) {
      setAppearance(cached.appearance);
      setRevision(cached.revision);
    } else {
      setAppearance(DEFAULT_APPEARANCE);
      setRevision(0);
    }
    setMediaReady(true);
  }, [userId]);

  useEffect(() => {
    let cancelled = false;
    void desktopWindowMaterials().then((materials) => {
      if (!cancelled) setNativeMaterials(materials);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const wantsArtwork = appearance.accent_from_artwork;
  const artworkColor = useArtworkSourceColor(artworkSrc, wantsArtwork);

  const environment = useMemo<ThemeEnvironment>(
    () => ({
      systemDark: mediaReady ? systemDark : true,
      reducedMotion,
      reducedTransparency,
      artworkColor,
      nativeMaterials,
    }),
    [
      artworkColor,
      mediaReady,
      nativeMaterials,
      reducedMotion,
      reducedTransparency,
      systemDark,
    ],
  );

  const theme = useMemo(
    () => resolveTheme(appearance, environment),
    [appearance, environment],
  );

  useLayoutEffect(() => {
    if (!mediaReady) return;
    applyThemeToDocument(theme);
  }, [mediaReady, theme]);

  // Remember it for the next load (both light and dark forms).
  useEffect(() => {
    if (!mediaReady) return;
    const timer = window.setTimeout(() => {
      try {
        const { systemDark: _ignored, ...rest } = environment;
        storage()?.setItem(
          THEME_CACHE_KEY,
          JSON.stringify(buildThemeCache(appearance, revision, rest, userId)),
        );
      } catch {
        // Private mode or full storage: the theme still applies.
      }
    }, 250);
    return () => window.clearTimeout(timer);
  }, [appearance, environment, mediaReady, revision, userId]);

  // Native window material in the desktop app (none elsewhere).
  const native = theme.glass.native;
  useEffect(() => {
    if (!nativeMaterials.length) return;
    let cancelled = false;
    void setDesktopWindowMaterial(native).then((applied) => {
      // The window could not show it (older Windows build, an Electron
      // without the API, a failure): paint the blurred backdrop instead of
      // leaving a page that expects a see-through window.
      if (!cancelled && native && applied !== native) setNativeMaterials([]);
    });
    return () => {
      cancelled = true;
    };
  }, [native, nativeMaterials.length]);

  const adopt = useCallback((next: Appearance, nextRevision: number) => {
    setAppearance(next);
    setRevision(nextRevision);
  }, []);

  // The profile's saved appearance, and changes made on other devices.
  const sync = useCallback(async () => {
    if (!userId || pending.current || saveTimer.current) return;
    try {
      const record = await getAppearance();
      if (pending.current || saveTimer.current) return;
      if (record.revision !== revisionRef.current) {
        adopt(record.appearance, record.revision);
      }
      setStatus("saved");
    } catch {
      setStatus((current) => (current === "saving" ? current : "error"));
    }
  }, [adopt, userId]);

  useEffect(() => {
    if (!userId) return;
    void sync();
    const interval = window.setInterval(() => void sync(), SYNC_INTERVAL_MS);
    const onFocus = () => {
      if (document.visibilityState === "visible") void sync();
    };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onFocus);
    return () => {
      window.clearInterval(interval);
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onFocus);
    };
  }, [sync, userId]);

  // Saves from the profile's other sessions, announced on the zone socket.
  useEffect(() => {
    if (!userId) return;
    return subscribeAppearanceChanges((revision) => {
      if (
        shouldFetchAnnounced(
          revision,
          revisionRef.current,
          Boolean(pending.current || saveTimer.current),
        )
      ) {
        void sync();
      }
    });
  }, [sync, userId]);

  const flush = useCallback(async () => {
    saveTimer.current = null;
    const patch = pending.current;
    if (!patch || !userId) return;
    pending.current = null;
    setStatus("saving");
    const attempt = async (base: Appearance, baseRevision: number) =>
      saveAppearance(normalizeAppearance({ ...base, ...patch }), baseRevision);
    try {
      let record;
      try {
        record = await attempt(appearanceRef.current, revisionRef.current);
      } catch (error) {
        const current = appearanceConflict(error);
        if (!current) throw error;
        // Another device saved first: keep its choices, apply ours on top.
        record = await attempt(current.appearance, current.revision);
      }
      if (!pending.current) adopt(record.appearance, record.revision);
      else setRevision(record.revision);
      setStatus("saved");
    } catch {
      setStatus("error");
    }
  }, [adopt, userId]);

  const update = useCallback(
    (patch: Partial<Appearance>) => {
      setAppearance((current) => normalizeAppearance({ ...current, ...patch }));
      pending.current = { ...(pending.current ?? {}), ...patch };
      if (!userId) return;
      setStatus("saving");
      if (saveTimer.current) window.clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(() => void flush(), SAVE_DELAY_MS);
    },
    [flush, userId],
  );

  const reset = useCallback(() => update(DEFAULT_APPEARANCE), [update]);

  const value = useMemo<AppearanceContextValue>(
    () => ({ appearance, theme, environment, revision, status, update, reset }),
    [appearance, environment, reset, revision, status, theme, update],
  );

  return (
    <AppearanceContext.Provider value={value}>
      {children}
    </AppearanceContext.Provider>
  );
}

export function useAppearance(): AppearanceContextValue {
  const context = useContext(AppearanceContext);
  if (!context) {
    throw new Error("useAppearance must be used within AppearanceProvider");
  }
  return context;
}

/** The appearance where a provider exists, else the defaults (login page). */
export function useOptionalAppearance(): AppearanceContextValue | null {
  return useContext(AppearanceContext);
}
