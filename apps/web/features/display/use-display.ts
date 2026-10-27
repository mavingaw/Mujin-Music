"use client";

import {
  DISPLAY_KEY_STORAGE,
  displayKeyFromHash,
  displayPollDelay,
  estimateDisplayPosition,
  getDisplayLyrics,
  getDisplayState,
  getDisplayWaveform,
  isDisplayUnpaired,
  parseLrc,
  pollDisplayPairing,
  startDisplayPairing,
  type DisplayPairing,
  type DisplayState,
  type DisplayWaveform,
  type TimedLyricLine,
} from "@parson/music-sdk";
import { useCallback, useEffect, useRef, useState } from "react";
import { readStored, suggestDisplayName, writeStored } from "./display-model";

/**
 * The screen's key: from a display link's `#key=` (then stored and wiped
 * from the address bar), else from this browser's storage.
 */
export function useDisplayKey() {
  const [key, setKeyState] = useState<string | null>(null);
  const [ready, setReady] = useState(false);
  useEffect(() => {
    const fromLink = displayKeyFromHash(window.location.hash);
    if (fromLink) {
      writeStored(DISPLAY_KEY_STORAGE, fromLink);
      window.history.replaceState(
        null,
        "",
        window.location.pathname + window.location.search,
      );
    }
    setKeyState(fromLink ?? readStored(DISPLAY_KEY_STORAGE));
    setReady(true);
  }, []);
  const setKey = useCallback((next: string | null) => {
    writeStored(DISPLAY_KEY_STORAGE, next);
    setKeyState(next);
  }, []);
  return { key, ready, setKey };
}

/**
 * Follows the zone: asks for the state every two seconds while playing
 * (faster near a track's end, slower when idle). A removed display goes
 * back to pairing; a lost connection keeps the last picture and retries.
 */
export function useDisplayState(key: string | null, onUnpaired: () => void) {
  const [state, setState] = useState<DisplayState | null>(null);
  const [receivedAt, setReceivedAt] = useState(0);
  const [offline, setOffline] = useState(false);
  const unpaired = useRef(onUnpaired);
  useEffect(() => {
    unpaired.current = onUnpaired;
  });
  useEffect(() => {
    if (!key) {
      setState(null);
      return;
    }
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let last: { state: DisplayState; at: number } | null = null;
    const tick = async () => {
      let delay = 5_000;
      try {
        const next = await getDisplayState(key);
        if (cancelled) return;
        const at = Date.now();
        last = { state: next, at };
        setState(next);
        setReceivedAt(at);
        setOffline(false);
        delay = displayPollDelay(
          next,
          estimateDisplayPosition(next, at, Date.now()),
        );
      } catch (error) {
        if (cancelled) return;
        if (isDisplayUnpaired(error)) {
          unpaired.current();
          return;
        }
        setOffline(true);
        if (last) delay = 5_000;
      }
      if (!cancelled) timer = setTimeout(tick, delay);
    };
    void tick();
    // A screen that wakes from sleep catches up at once.
    const onVisible = () => {
      if (document.visibilityState !== "visible") return;
      if (timer) clearTimeout(timer);
      void tick();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [key]);
  return { offline, receivedAt, state };
}

/** Playback position now, ticking four times a second while playing. */
export function useDisplayPosition(
  state: DisplayState | null,
  receivedAt: number,
) {
  const [now, setNow] = useState(() => Date.now());
  const playing = state?.zone?.status === "playing";
  useEffect(() => {
    if (!playing) return;
    const timer = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(timer);
  }, [playing]);
  if (!state) return 0;
  return estimateDisplayPosition(state, receivedAt, Math.max(now, receivedAt));
}

/** Wall-clock time, refreshed each minute on the minute. */
export function useMinuteClock() {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout>;
    const schedule = () => {
      const current = new Date();
      setNow(current);
      timer = setTimeout(schedule, 60_000 - (current.getTime() % 60_000) + 50);
    };
    schedule();
    return () => clearTimeout(timer);
  }, []);
  return now;
}

/** Timed lines (and plain words) of the track the display shows. */
export function useDisplayLyrics(
  key: string | null,
  songId: string | null,
  enabled: boolean,
) {
  const [lyrics, setLyrics] = useState<{
    songId: string;
    timed: TimedLyricLine[];
    plain: string | null;
  } | null>(null);
  useEffect(() => {
    if (!key || !songId || !enabled) return;
    let cancelled = false;
    getDisplayLyrics(key, songId)
      .then((result) => {
        if (cancelled) return;
        setLyrics({
          songId,
          timed: parseLrc(result.syncedLyrics),
          plain: result.plainLyrics,
        });
      })
      .catch(() => {
        if (!cancelled) setLyrics({ songId, timed: [], plain: null });
      });
    return () => {
      cancelled = true;
    };
  }, [enabled, key, songId]);
  return lyrics && lyrics.songId === songId && enabled
    ? lyrics
    : { songId, timed: [] as TimedLyricLine[], plain: null };
}

/**
 * The track's waveform. The server works it out on first request (a few
 * seconds); until then the seek bar is a plain line.
 */
export function useDisplayWaveform(
  key: string | null,
  songId: string | null,
  enabled: boolean,
) {
  const [waveform, setWaveform] = useState<{
    songId: string;
    value: DisplayWaveform;
  } | null>(null);
  useEffect(() => {
    if (!key || !songId || !enabled) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let attempts = 0;
    const ask = () => {
      attempts += 1;
      getDisplayWaveform(key, songId)
        .then((value) => {
          if (cancelled) return;
          if (value) setWaveform({ songId, value });
          else if (attempts < 12) timer = setTimeout(ask, 3_000);
        })
        .catch(() => {});
    };
    ask();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [enabled, key, songId]);
  return waveform && waveform.songId === songId ? waveform.value : null;
}

/**
 * Pairing by code: starts a pairing, shows its code, renews it before it
 * expires and hands back the key once a signed-in app approves it.
 */
export function useDisplayPairing(
  active: boolean,
  onPaired: (key: string) => void,
) {
  const [pairing, setPairing] = useState<
    (DisplayPairing & { expiresAt: number }) | null
  >(null);
  const [error, setError] = useState<string | null>(null);
  const paired = useRef(onPaired);
  useEffect(() => {
    paired.current = onPaired;
  });
  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let current: (DisplayPairing & { expiresAt: number }) | null = null;
    const start = async () => {
      try {
        const started = await startDisplayPairing(
          suggestDisplayName(navigator.userAgent),
        );
        if (cancelled) return;
        current = {
          ...started,
          expiresAt: Date.now() + started.expires_in * 1000,
        };
        setPairing(current);
        setError(null);
        timer = setTimeout(poll, 2_000);
      } catch {
        if (cancelled) return;
        setError("Can’t reach Mujin Music. Retrying…");
        timer = setTimeout(start, 5_000);
      }
    };
    const poll = async () => {
      if (!current) return start();
      if (Date.now() >= current.expiresAt - 5_000) return start();
      try {
        const result = await pollDisplayPairing(current);
        if (cancelled) return;
        if (result.status === "approved") {
          paired.current(result.key);
          return;
        }
        if (result.status === "expired") return start();
        setError(null);
      } catch {
        if (cancelled) return;
        setError("Can’t reach Mujin Music. Retrying…");
      }
      timer = setTimeout(poll, 2_000);
    };
    void start();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [active]);
  return { error, pairing };
}

/**
 * True for three seconds after the pointer moves or a key is pressed: the
 * cursor and the small toolbar show only then.
 */
export function usePointerActivity(timeoutMs = 3_000) {
  const [active, setActive] = useState(false);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const wake = () => {
      setActive(true);
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => setActive(false), timeoutMs);
    };
    window.addEventListener("pointermove", wake);
    window.addEventListener("pointerdown", wake);
    window.addEventListener("keydown", wake);
    return () => {
      if (timer) clearTimeout(timer);
      window.removeEventListener("pointermove", wake);
      window.removeEventListener("pointerdown", wake);
      window.removeEventListener("keydown", wake);
    };
  }, [timeoutMs]);
  return active;
}

/** Keeps the screen awake while the display is visible, where allowed. */
export function useWakeLock(enabled: boolean) {
  useEffect(() => {
    if (!enabled || !("wakeLock" in navigator)) return;
    let lock: { release: () => Promise<void> } | null = null;
    let cancelled = false;
    const request = async () => {
      try {
        const sentinel = await (
          navigator as Navigator & {
            wakeLock: {
              request: (
                type: "screen",
              ) => Promise<{ release: () => Promise<void> }>;
            };
          }
        ).wakeLock.request("screen");
        if (cancelled) void sentinel.release();
        else lock = sentinel;
      } catch {
        // Not allowed here (no user gesture yet, or a TV browser without it).
      }
    };
    const onVisible = () => {
      if (document.visibilityState === "visible") void request();
    };
    void request();
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      cancelled = true;
      document.removeEventListener("visibilitychange", onVisible);
      void lock?.release().catch(() => {});
    };
  }, [enabled]);
}

/** Whether the viewer asked for less motion. */
export function useReducedMotion() {
  const [reduced, setReduced] = useState(false);
  useEffect(() => {
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    setReduced(query.matches);
    const onChange = () => setReduced(query.matches);
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);
  return reduced;
}
