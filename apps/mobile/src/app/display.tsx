import {
  activeLyricIndex,
  createDisplay,
  displayAssetURL,
  displayCaption,
  displayPollDelay,
  displayQualityChip,
  displayTrackLine,
  estimateDisplayPosition,
  formatDisplayTime,
  getDisplayLyrics,
  getDisplayState,
  getDisplayWaveform,
  isDisplayUnpaired,
  lyricLineEmphasis,
  parseLrc,
  waveformBars,
  type DisplayQualityChip,
  type DisplayState,
  type DisplayWaveform,
  type TimedLyricLine,
} from "@parson/music-sdk";
import { Image } from "expo-image";
import { LinearGradient } from "expo-linear-gradient";
import { useLocalSearchParams, useRouter } from "expo-router";
import { StatusBar } from "expo-status-bar";
import { X } from "lucide-react-native";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  Pressable,
  StyleSheet,
  Text,
  useWindowDimensions,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import { fonts, palette } from "@/constants/colors";
import {
  displayKeyStorageName,
  displayMetrics,
  visibleLyricRange,
} from "@/lib/display";
import { getSecureItem, setSecureItem } from "@/lib/secure-storage";
import { useZones } from "@/providers/zones-provider";

const LIGHT: Record<DisplayQualityChip["light"], string> = {
  lossless: palette.accent,
  enhanced: "#8FC3E0",
  high: palette.qualityHigh,
  low: palette.qualityLow,
  unknown: palette.tertiary,
};

/**
 * This key, made once per zone and kept in secure storage, lets the screen
 * read the zone's now-playing state like any TV display does.
 */
function useDisplayKey(zoneId: string | null, zoneName: string | null) {
  const [key, setKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let cancelled = false;
    const storage = displayKeyStorageName(zoneId);
    void (async () => {
      const stored = attempt === 0 ? await getSecureItem(storage) : null;
      if (stored) {
        if (!cancelled) setKey(stored);
        return;
      }
      try {
        const created = await createDisplay({
          name: zoneName ? `${zoneName} · this screen` : "This screen",
          zone_id: zoneId,
        });
        await setSecureItem(storage, created.key);
        if (!cancelled) setKey(created.key);
      } catch {
        if (!cancelled) setError("Could not start the display.");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [attempt, zoneId, zoneName]);
  return { error, key, renew: () => setAttempt((value) => value + 1) };
}

function useDisplayState(key: string | null, onUnpaired: () => void) {
  const [state, setState] = useState<DisplayState | null>(null);
  const [receivedAt, setReceivedAt] = useState(0);
  const unpaired = useRef(onUnpaired);
  useEffect(() => {
    unpaired.current = onUnpaired;
  });
  useEffect(() => {
    if (!key) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      let delay = 5_000;
      try {
        const next = await getDisplayState(key);
        if (cancelled) return;
        const at = Date.now();
        setState(next);
        setReceivedAt(at);
        delay = displayPollDelay(next, estimateDisplayPosition(next, at, at));
      } catch (error) {
        if (cancelled) return;
        if (isDisplayUnpaired(error)) {
          unpaired.current();
          return;
        }
      }
      if (!cancelled) timer = setTimeout(tick, delay);
    };
    void tick();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [key]);
  return { receivedAt, state };
}

function usePosition(state: DisplayState | null, receivedAt: number) {
  const [now, setNow] = useState(() => Date.now());
  const playing = state?.zone?.status === "playing";
  useEffect(() => {
    if (!playing) return;
    const timer = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(timer);
  }, [playing]);
  return state
    ? estimateDisplayPosition(state, receivedAt, Math.max(now, receivedAt))
    : 0;
}

function useTrackExtras(
  key: string | null,
  songId: string | null,
  live: boolean,
  lyricsOn: boolean,
) {
  const [lyrics, setLyrics] = useState<{
    songId: string;
    lines: TimedLyricLine[];
  } | null>(null);
  const [waveform, setWaveform] = useState<{
    songId: string;
    value: DisplayWaveform;
  } | null>(null);
  useEffect(() => {
    if (!key || !songId || live || !lyricsOn) return;
    let cancelled = false;
    getDisplayLyrics(key, songId)
      .then((result) => {
        if (!cancelled)
          setLyrics({ songId, lines: parseLrc(result.syncedLyrics) });
      })
      .catch(() => {
        if (!cancelled) setLyrics({ songId, lines: [] });
      });
    return () => {
      cancelled = true;
    };
  }, [key, live, lyricsOn, songId]);
  useEffect(() => {
    if (!key || !songId || live) return;
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
  }, [key, live, songId]);
  return {
    lines: lyrics?.songId === songId && lyricsOn ? lyrics.lines : [],
    waveform: waveform?.songId === songId ? waveform.value : null,
  };
}

function useClock() {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const timer = setInterval(() => setNow(new Date()), 15_000);
    return () => clearInterval(timer);
  }, []);
  return now.toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
}

/**
 * Display mode: this tablet or phone shows what a zone plays, full screen,
 * like a TV display. Tap once for the close button.
 */
export default function DisplayScreen() {
  const router = useRouter();
  const params = useLocalSearchParams<{ zone?: string }>();
  const zones = useZones();
  const zoneId = params.zone && params.zone !== "follow" ? params.zone : null;
  const zoneName = zoneId
    ? (zones.zones.find((zone) => zone.id === zoneId)?.name ?? null)
    : null;
  const { error, key, renew } = useDisplayKey(zoneId, zoneName);
  const { receivedAt, state } = useDisplayState(key, renew);
  const position = usePosition(state, receivedAt);
  const track = state?.status === "playing" ? state.track : null;
  const { lines, waveform } = useTrackExtras(
    key,
    track?.song_id ?? null,
    Boolean(track?.live),
    state?.display.lyrics !== "off",
  );
  const clock = useClock();
  const insets = useSafeAreaInsets();
  const { width, height } = useWindowDimensions();
  const metrics = displayMetrics(width, height);
  const [chrome, setChrome] = useState(false);
  useEffect(() => {
    if (!chrome) return;
    const timer = setTimeout(() => setChrome(false), 4_000);
    return () => clearTimeout(timer);
  }, [chrome]);
  const timed = lines.some((line) => line.text.trim());
  const lyrics = Boolean(track && timed && !track.live);
  const cover = displayAssetURL(track?.cover_url);
  const photo = displayAssetURL(track?.artist_image_url);
  const chip = displayQualityChip(track, state?.zone ?? null);
  const duration = track?.duration_ms || state?.zone?.duration_ms || 0;
  const progress = duration > 0 ? Math.min(1, position / duration) : 0;
  const bars = useMemo(
    () => waveformBars(waveform, Math.max(40, Math.floor(width / 9))),
    [waveform, width],
  );
  const trackLine = track ? displayTrackLine(track) : "";

  return (
    <Pressable
      accessibilityLabel="Now playing display. Tap for options."
      onPress={() => setChrome(true)}
      style={styles.page}
    >
      <StatusBar hidden />
      {track ? (
        <>
          {cover ? (
            <Image
              blurRadius={60}
              contentFit="cover"
              source={{ uri: cover }}
              style={[StyleSheet.absoluteFill, { opacity: 0.55 }]}
              transition={900}
            />
          ) : null}
          {photo ? (
            <Image
              contentFit="cover"
              contentPosition={{ top: "25%", left: "70%" }}
              source={{ uri: photo }}
              style={[
                styles.photo,
                metrics.landscape
                  ? { width: "64%" }
                  : { height: "62%", width: "100%" },
              ]}
              transition={900}
            />
          ) : null}
          <LinearGradient
            colors={["rgba(5,5,6,0.95)", "rgba(5,5,6,0.25)", "rgba(5,5,6,0)"]}
            end={metrics.landscape ? { x: 0.75, y: 0.5 } : { x: 0.5, y: 0.1 }}
            start={
              metrics.landscape ? { x: 0.25, y: 0.5 } : { x: 0.5, y: 0.75 }
            }
            style={StyleSheet.absoluteFill}
          />
          <LinearGradient
            colors={["rgba(5,5,6,0)", "rgba(5,5,6,0.92)"]}
            start={{ x: 0.5, y: 0.35 }}
            end={{ x: 0.5, y: 1 }}
            style={StyleSheet.absoluteFill}
          />
          {lyrics ? (
            <View style={[StyleSheet.absoluteFill, styles.lyricsDim]} />
          ) : null}
        </>
      ) : null}
      <View
        style={[
          styles.content,
          {
            paddingBottom: Math.max(insets.bottom, 16) + 8,
            paddingLeft: Math.max(insets.left, metrics.gutter),
            paddingRight: Math.max(insets.right, metrics.gutter),
            paddingTop: Math.max(insets.top, 14) + 4,
          },
        ]}
      >
        <View style={styles.top}>
          <Text numberOfLines={1} style={styles.zone}>
            {state?.zone && track ? state.zone.name.toUpperCase() : ""}
            {state?.zone && track && state.zone.status !== "playing"
              ? "  ·  PAUSED"
              : ""}
          </Text>
          {track ? (
            <Text style={[styles.clock, { fontSize: metrics.clock }]}>
              {clock}
            </Text>
          ) : null}
        </View>

        {!track ? (
          <View style={styles.center}>
            <Text style={[styles.bigClock, { fontSize: metrics.title * 1.8 }]}>
              {clock}
            </Text>
            <Text style={styles.caption}>
              {error ?? (key ? displayCaption(state) : "Starting the display…")}
            </Text>
          </View>
        ) : (
          <>
            <View style={styles.middle}>
              {lyrics ? (
                <Lyrics
                  lines={lines}
                  position={position}
                  size={metrics.lyric}
                />
              ) : null}
            </View>
            <View style={styles.meta}>
              {cover ? (
                <Image
                  accessibilityIgnoresInvertColors
                  contentFit="cover"
                  source={{ uri: cover }}
                  style={[
                    styles.cover,
                    {
                      height: lyrics ? metrics.coverInLyrics : metrics.cover,
                      width: lyrics ? metrics.coverInLyrics : metrics.cover,
                    },
                  ]}
                  transition={600}
                />
              ) : null}
              <View style={styles.metaText}>
                {trackLine ? (
                  <Text style={styles.eyebrow}>
                    {track.live ? "LIVE RADIO" : trackLine.toUpperCase()}
                  </Text>
                ) : null}
                <Text
                  numberOfLines={2}
                  style={[
                    styles.title,
                    {
                      fontSize: lyrics ? metrics.titleInLyrics : metrics.title,
                    },
                  ]}
                >
                  {track.title}
                </Text>
                <Text
                  numberOfLines={1}
                  style={[
                    styles.artist,
                    { fontSize: metrics.artist * (lyrics ? 0.8 : 1) },
                  ]}
                >
                  {track.artist}
                </Text>
                {chip ? (
                  <View style={styles.chip}>
                    <View
                      style={[
                        styles.light,
                        { backgroundColor: LIGHT[chip.light] },
                      ]}
                    />
                    <Text numberOfLines={1} style={styles.chipText}>
                      {[chip.format, chip.tier].filter(Boolean).join(" · ")}
                    </Text>
                    {chip.path ? (
                      <Text
                        style={[styles.chipPath, { color: LIGHT[chip.light] }]}
                      >
                        {chip.path}
                      </Text>
                    ) : null}
                  </View>
                ) : null}
              </View>
            </View>
            {track.live ? null : (
              <View style={styles.seek}>
                <Text style={styles.time}>{formatDisplayTime(position)}</Text>
                <View style={styles.bars}>
                  {bars.length ? (
                    bars.map((level, index) => (
                      <View
                        key={index}
                        style={[
                          styles.bar,
                          {
                            backgroundColor:
                              (index + 0.5) / bars.length <= progress
                                ? palette.text
                                : "rgba(242,238,230,0.18)",
                            height: `${Math.max(10, level * 100)}%`,
                          },
                        ]}
                      />
                    ))
                  ) : (
                    <View style={styles.line}>
                      <View
                        style={[
                          styles.lineFill,
                          { width: `${progress * 100}%` },
                        ]}
                      />
                    </View>
                  )}
                </View>
                <Text style={[styles.time, { color: palette.tertiary }]}>
                  {formatDisplayTime(duration)}
                </Text>
              </View>
            )}
          </>
        )}
      </View>
      {chrome ? (
        <Pressable
          accessibilityLabel="Close display"
          accessibilityRole="button"
          onPress={() => router.back()}
          style={[
            styles.close,
            {
              top: Math.max(insets.top, 14) + 44,
              right: Math.max(insets.right, metrics.gutter),
            },
          ]}
        >
          <X color={palette.text} size={22} />
        </Pressable>
      ) : null}
    </Pressable>
  );
}

function Lyrics({
  lines,
  position,
  size,
}: {
  lines: TimedLyricLine[];
  position: number;
  size: number;
}) {
  const active = activeLyricIndex(lines, position / 1000);
  const { start, end } = visibleLyricRange(lines.length, active);
  return (
    <View accessibilityLabel="Lyrics" style={styles.lyrics}>
      {lines.slice(start, end).map((line, offset) => {
        const index = start + offset;
        const emphasis = lyricLineEmphasis(index, active);
        return (
          <Text
            key={`${index}-${line.time}`}
            style={[
              styles.lyric,
              {
                color:
                  emphasis.state === "active"
                    ? palette.text
                    : palette.secondary,
                fontSize: size,
                lineHeight: size * 1.15,
                opacity: emphasis.opacity,
              },
            ]}
          >
            {line.text || "•  •  •"}
          </Text>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  page: { backgroundColor: "#050506", flex: 1 },
  photo: { bottom: 0, position: "absolute", right: 0, top: 0 },
  lyricsDim: { backgroundColor: "rgba(0,0,0,0.45)" },
  content: { flex: 1 },
  top: {
    alignItems: "flex-start",
    flexDirection: "row",
    justifyContent: "space-between",
  },
  zone: {
    color: palette.secondary,
    flex: 1,
    fontSize: 12,
    fontWeight: "600",
    letterSpacing: 1.6,
  },
  clock: { color: palette.text, fontFamily: fonts.display },
  center: { alignItems: "center", flex: 1, justifyContent: "center" },
  bigClock: { color: palette.text, fontFamily: fonts.display },
  caption: {
    color: palette.secondary,
    fontSize: 17,
    marginTop: 18,
    textAlign: "center",
  },
  middle: { flex: 1, justifyContent: "center" },
  lyrics: { gap: 6 },
  lyric: { fontFamily: fonts.display },
  meta: { alignItems: "flex-end", flexDirection: "row", gap: 20 },
  cover: { borderRadius: 6 },
  metaText: { flex: 1, minWidth: 0 },
  eyebrow: {
    color: palette.tertiary,
    fontSize: 11,
    fontWeight: "600",
    letterSpacing: 1.6,
    marginBottom: 8,
  },
  title: { color: palette.text, fontFamily: fonts.display },
  artist: { color: palette.secondary, marginTop: 6 },
  chip: {
    alignItems: "center",
    alignSelf: "flex-start",
    backgroundColor: "rgba(0,0,0,0.35)",
    borderColor: "rgba(242,238,230,0.12)",
    borderRadius: 999,
    borderWidth: 1,
    flexDirection: "row",
    gap: 8,
    marginTop: 14,
    maxWidth: "100%",
    paddingHorizontal: 12,
    paddingVertical: 6,
  },
  light: { borderRadius: 999, height: 8, width: 8 },
  chipText: {
    color: palette.text,
    flexShrink: 1,
    fontFamily: "monospace",
    fontSize: 13,
  },
  chipPath: { fontSize: 13, fontWeight: "600" },
  seek: { alignItems: "center", flexDirection: "row", gap: 12, marginTop: 22 },
  time: {
    color: palette.secondary,
    fontFamily: "monospace",
    fontSize: 13,
    minWidth: 40,
  },
  bars: {
    alignItems: "center",
    flex: 1,
    flexDirection: "row",
    gap: 2,
    height: 36,
  },
  bar: { borderRadius: 999, flex: 1 },
  line: {
    backgroundColor: "rgba(242,238,230,0.15)",
    borderRadius: 2,
    flex: 1,
    height: 3,
  },
  lineFill: { backgroundColor: palette.text, borderRadius: 2, height: 3 },
  close: {
    alignItems: "center",
    backgroundColor: "rgba(0,0,0,0.55)",
    borderColor: "rgba(242,238,230,0.12)",
    borderRadius: 999,
    borderWidth: 1,
    height: 44,
    justifyContent: "center",
    position: "absolute",
    width: 44,
  },
});
