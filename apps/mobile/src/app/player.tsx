import Slider from "@react-native-community/slider";
/* eslint-disable react-hooks/immutability -- Reanimated SharedValues are mutated only inside gesture worklets. */
import {
  activeLyricIndex,
  addFavoriteSong,
  findLyrics,
  lyricLineEmphasis,
  lyricsEmptyMessage,
  lyricsFailureKind,
  lyricsSourceLabel,
  parseLrc,
  type LyricsFailure,
  type LyricsSourceKind,
  isFavoriteSong,
  radioStationLabel,
  removeFavoriteSong,
  heartState,
  getLiveNowPlaying,
  isLiveRadioId,
  isStreamingId,
  LIVE_TITLE_POLL_MS,
  liveStationIdFromSongId,
  liveTitleText,
  type SignalPath,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { LinearGradient } from "expo-linear-gradient";
import { Image } from "expo-image";
import { useLocalSearchParams, useRouter } from "expo-router";
import { StreamDeliveryLine } from "@/components/streaming-ui";
import {
  ChevronDown,
  ChevronLeft,
  Disc3,
  Download,
  ListEnd,
  ListPlus,
  MoreHorizontal,
  Play,
  Repeat,
  Repeat1,
  Shuffle,
  SkipBack,
  SkipForward,
  Ban,
  Heart,
  Gauge,
  AudioLines,
  TimerReset,
  X,
  UserRound,
  Tv,
} from "lucide-react-native";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  FlatList,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  useWindowDimensions,
  View,
} from "react-native";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import Animated, {
  runOnJS,
  useAnimatedStyle,
  useSharedValue,
  withSpring,
  withTiming,
} from "react-native-reanimated";
import { SafeAreaView } from "react-native-safe-area-context";

import { Artwork } from "@/components/artwork";
import { ActionDrawer, DrawerAction } from "@/components/action-drawer";
import { useZones } from "@/providers/zones-provider";
import { SongRow } from "@/components/music-ui";
import { fonts, palette } from "@/constants/colors";
import {
  playerAudioPresets,
  usePlayer,
  usePlayerPosition,
  type AudioPreset,
} from "@/providers/player-provider";
import { useZonePlayback } from "@/hooks/use-zone-playback";
import {
  downloadSong,
  isSongDownloaded,
  removeDownload,
  useDownloadsRevision,
} from "@/lib/downloads";
import { PlaylistPicker } from "@/components/playlist-picker";
import { PauseGlyph } from "@/components/pause-glyph";
import { useSession } from "@/providers/session-provider";
import { BanAction, useRadioBans } from "@/components/radio-bans";
import { CastOutputButton } from "@/components/cast-output-button";
import {
  RadioNotices,
  RadioQueueHeader,
  RadioThumbs,
  StartRadioAction,
} from "@/components/radio-ui";
import { SignalPathBadge, SignalPathContent } from "@/components/signal-path";
import { ServerOutputs } from "@/components/server-outputs";
import { dspAvailable } from "@/lib/signal-path";
import { useSignalPath } from "@/hooks/use-signal-path";
import { LiveTag, StationLogo } from "@/components/live-radio-ui";
import { ZoneButton } from "@/components/zone-sheet";
import { imageRequestHeaders, imageUrl } from "@/lib/runtime";
import {
  immediateBorderlessPressFeedback,
  immediatePressFeedback,
} from "@/lib/press-feedback";

type Panel = "player" | "queue" | "lyrics";
const AnimatedLinearGradient = Animated.createAnimatedComponent(LinearGradient);
const presetIcons = {
  original: AudioLines,
  slow: TimerReset,
  "deep-slow": TimerReset,
  faster: Gauge,
} satisfies Record<AudioPreset, typeof AudioLines>;

const time = (seconds: number) =>
  `${Math.floor(seconds / 60)}:${Math.floor(seconds % 60)
    .toString()
    .padStart(2, "0")}`;

type Player = ReturnType<typeof usePlayer>;
type PlayerSong = NonNullable<Player["current"]>;
type Router = ReturnType<typeof useRouter>;
type SyncedLine = { time: number; text: string };

/** "The Open Goldberg Variations · 2012 · Bach" under the artist. */
function nowPlayingMeta(song: {
  album_object?: { name?: string; first_release_date?: string } | null;
  credits?: { role: string; name: string }[];
}) {
  const year = /^\d{4}/.exec(song.album_object?.first_release_date ?? "")?.[0];
  const composer = song.credits?.find(
    (credit) => credit.role === "composer",
  )?.name;
  return [
    song.album_object?.name,
    year && year !== "0000" ? year : null,
    composer,
  ]
    .filter(Boolean)
    .join(" · ");
}

function PlayerHeader({
  panel,
  onBack,
  onActions,
}: {
  panel: Panel;
  onBack: () => void;
  onActions: () => void;
}) {
  return (
    <View style={styles.header}>
      <Pressable
        {...immediateBorderlessPressFeedback}
        accessibilityLabel={
          panel === "player" ? "Close player" : "Back to player"
        }
        accessibilityRole="button"
        hitSlop={13}
        onPress={onBack}
      >
        {panel === "player" ? (
          <ChevronDown color="white" size={29} />
        ) : (
          <ChevronLeft color="white" size={29} />
        )}
      </Pressable>
      <Text style={styles.panelHeader}>
        {panel === "player" ? "" : panel === "lyrics" ? "Lyrics" : "Queue"}
      </Text>
      {panel === "player" ? (
        <Pressable
          {...immediateBorderlessPressFeedback}
          accessibilityLabel="More player actions"
          accessibilityRole="button"
          hitSlop={13}
          onPress={onActions}
        >
          <MoreHorizontal color="white" size={25} />
        </Pressable>
      ) : (
        <View style={{ width: 25 }} />
      )}
    </View>
  );
}

function NowPlayingPanel({
  player,
  song,
  router,
  favorite,
  banned,
  onToggleFavorite,
  onQueue,
  onLyrics,
  onSound,
  onSignalPath,
  signalPath,
  notice,
  online,
}: {
  player: Player;
  song: PlayerSong;
  router: Router;
  favorite: boolean;
  /** Third heart state, as in Roon: Radio and Shuffle never pick it. */
  banned: boolean;
  onToggleFavorite: () => void;
  onQueue: () => void;
  onLyrics: () => void;
  onSound: () => void;
  onSignalPath: () => void;
  signalPath: SignalPath | null;
  notice?: string | null;
  online: boolean;
}) {
  const presetLabel = playerAudioPresets.find(
    (preset) => preset.id === player.audioPreset,
  )?.label;
  const live = isLiveRadioId(song.id);
  return (
    <View style={styles.player}>
      {live ? (
        <View style={styles.art}>
          <StationLogo
            rounded={14}
            size={330}
            station={{
              name: song.name,
              logo_url: song.album_object?.cover_url || null,
            }}
          />
        </View>
      ) : (
        <Artwork
          path={song.album_object?.cover_url}
          size={330}
          rounded={14}
          style={styles.art}
        />
      )}
      <View style={styles.songInfo}>
        <View style={{ flex: 1 }}>
          {player.radio.currentIsPick ? (
            <Text numberOfLines={1} style={styles.radioLabel}>
              {player.radio.seed && player.radio.seed.kind !== "song"
                ? radioStationLabel(player.radio.seed)
                : `Radio · ${radioStationLabel(player.radio.seed)}`}
            </Text>
          ) : null}
          <Text
            accessibilityLabel={`View album ${song.album_object?.name ?? song.name}`}
            accessibilityRole={song.album_object?.id ? "button" : undefined}
            numberOfLines={1}
            style={styles.title}
            onPress={() => {
              const station = liveStationIdFromSongId(song.id);
              if (station) router.replace(`/radio/station/${station}`);
              else if (song.album_object?.id)
                router.replace(`/album/${song.album_object.id}`);
            }}
          >
            {song.name}
          </Text>
          <Text
            accessibilityLabel={`View artist ${song.artist}`}
            accessibilityRole={song.artist_object?.id ? "button" : undefined}
            numberOfLines={1}
            style={styles.artist}
            onPress={() =>
              song.artist_object?.id &&
              router.replace(`/artist/${song.artist_object.id}`)
            }
          >
            {live ? (
              <LiveTitle fallback={song.artist} songId={song.id} />
            ) : (
              song.artist
            )}
          </Text>
          {!live && nowPlayingMeta(song) ? (
            <Text numberOfLines={1} style={styles.meta}>
              {nowPlayingMeta(song)}
            </Text>
          ) : null}
          {live ? (
            <View style={styles.liveRow}>
              <LiveTag />
              <SignalPathBadge onPress={onSignalPath} path={signalPath} />
            </View>
          ) : isStreamingId(song.id) ? (
            <StreamDeliveryLine songId={song.id} />
          ) : (
            <SignalPathBadge onPress={onSignalPath} path={signalPath} />
          )}
        </View>
        {live ? null : (
          <Pressable
            {...immediateBorderlessPressFeedback}
            accessibilityLabel={
              !online
                ? "Liked songs unavailable offline"
                : banned
                  ? "Banned from Radio and Shuffle. Unban"
                  : favorite
                    ? "Remove from liked songs and ban from Radio"
                    : "Add to liked songs"
            }
            accessibilityRole="button"
            disabled={!online}
            hitSlop={12}
            style={!online ? styles.disabledControl : undefined}
            onPress={onToggleFavorite}
          >
            {banned ? (
              <Ban color={palette.danger} size={25} />
            ) : (
              <Heart
                color={favorite ? palette.danger : "white"}
                fill={favorite ? palette.danger : "transparent"}
                size={25}
              />
            )}
          </Pressable>
        )}
      </View>
      {online && !live ? (
        <View style={styles.feedbackRow}>
          <RadioThumbs index={player.currentIndex} name={song.name} />
        </View>
      ) : null}
      {live ? (
        <View style={styles.liveBar}>
          <View style={styles.liveLine} />
        </View>
      ) : (
        <PlaybackTimeline onSeek={player.seek} />
      )}
      {online ? <RadioNotices /> : null}
      {notice ? (
        <Text accessibilityRole="alert" style={styles.playbackError}>
          {notice}
        </Text>
      ) : null}
      <View style={styles.controls}>
        <Pressable
          {...immediateBorderlessPressFeedback}
          accessibilityLabel={
            player.shuffle ? "Turn shuffle off" : "Turn shuffle on"
          }
          accessibilityRole="button"
          accessibilityState={{ selected: player.shuffle }}
          hitSlop={12}
          onPress={player.toggleShuffle}
        >
          <Shuffle color={player.shuffle ? "white" : palette.muted} size={24} />
        </Pressable>
        <Pressable
          {...immediateBorderlessPressFeedback}
          accessibilityLabel="Previous song"
          accessibilityRole="button"
          hitSlop={14}
          onPress={player.previous}
        >
          <SkipBack color="white" fill="white" size={31} />
        </Pressable>
        <Pressable
          {...immediatePressFeedback}
          accessibilityLabel={player.isPlaying ? "Pause" : "Play"}
          accessibilityRole="button"
          style={styles.playButton}
          onPress={player.toggle}
        >
          {player.isPlaying ? (
            <PauseGlyph color={palette.accentText} size={30} />
          ) : (
            <Play
              color={palette.accentText}
              fill={palette.accentText}
              size={34}
            />
          )}
        </Pressable>
        <Pressable
          {...immediateBorderlessPressFeedback}
          accessibilityLabel="Next song"
          accessibilityRole="button"
          hitSlop={14}
          onPress={player.next}
        >
          <SkipForward color="white" fill="white" size={31} />
        </Pressable>
        <Pressable
          {...immediateBorderlessPressFeedback}
          accessibilityLabel={`Repeat ${player.repeat}`}
          accessibilityRole="button"
          hitSlop={12}
          onPress={player.cycleRepeat}
        >
          {player.repeat === "one" ? (
            <Repeat1 color="white" size={24} />
          ) : (
            <Repeat
              color={player.repeat === "all" ? "white" : palette.muted}
              size={24}
            />
          )}
        </Pressable>
      </View>
      {!live && player.queue[player.currentIndex + 1] ? (
        <Pressable
          {...immediatePressFeedback}
          accessibilityHint="Opens the queue"
          accessibilityLabel={`Up next: ${player.queue[player.currentIndex + 1]!.name}`}
          accessibilityRole="button"
          onPress={onQueue}
          style={styles.upNext}
        >
          <Artwork
            path={
              player.queue[player.currentIndex + 1]!.album_object?.cover_url
            }
            rounded={4}
            size={38}
          />
          <View style={{ flex: 1, minWidth: 0 }}>
            <Text style={styles.upNextLabel}>UP NEXT</Text>
            <Text numberOfLines={1} style={styles.upNextTitle}>
              {player.queue[player.currentIndex + 1]!.name}
              <Text style={styles.upNextArtist}>
                {"  ·  "}
                {player.queue[player.currentIndex + 1]!.artist}
              </Text>
            </Text>
          </View>
        </Pressable>
      ) : null}
      <View style={styles.panelButtons}>
        <View style={styles.castButton}>
          <CastOutputButton />
        </View>
        <View style={styles.castButton}>
          <ZoneButton />
        </View>
        <Pressable
          {...immediatePressFeedback}
          accessibilityRole="button"
          style={styles.panelButton}
          onPress={onQueue}
        >
          <Text style={styles.panelText}>Queue</Text>
        </Pressable>
        <Pressable
          {...immediatePressFeedback}
          accessibilityRole="button"
          style={styles.panelButton}
          onPress={onLyrics}
        >
          <Text style={styles.panelText}>Lyrics</Text>
        </Pressable>
        {player.audioPresetsEnabled ? (
          <Pressable
            {...immediatePressFeedback}
            accessibilityLabel="Sound settings"
            accessibilityRole="button"
            style={styles.panelButton}
            onPress={onSound}
          >
            <Text style={styles.panelText}>
              {player.audioPreset === "original" ? "Sound" : presetLabel}
            </Text>
          </Pressable>
        ) : null}
      </View>
    </View>
  );
}

function PlaybackTimeline({ onSeek }: { onSeek: (seconds: number) => void }) {
  const { currentTime, duration } = usePlayerPosition();
  const [scrubPosition, setScrubPosition] = useState<number | null>(null);
  const settlingSeek = useRef<{ target: number; expiresAt: number } | null>(
    null,
  );
  useEffect(() => {
    const pending = settlingSeek.current;
    if (!pending) return;
    if (
      Math.abs(currentTime - pending.target) < 0.65 ||
      Date.now() >= pending.expiresAt
    ) {
      settlingSeek.current = null;
      setScrubPosition(null);
      return;
    }
    const timeout = setTimeout(
      () => {
        settlingSeek.current = null;
        setScrubPosition(null);
      },
      Math.max(0, pending.expiresAt - Date.now()),
    );
    return () => clearTimeout(timeout);
  }, [currentTime]);
  const shownTime = scrubPosition ?? currentTime;
  return (
    <>
      <View style={styles.sliderHitbox}>
        <Slider
          accessibilityLabel="Playback position"
          minimumValue={0}
          maximumValue={Math.max(1, duration)}
          value={shownTime}
          onSlidingStart={setScrubPosition}
          onValueChange={setScrubPosition}
          onSlidingComplete={(target) => {
            setScrubPosition(target);
            settlingSeek.current = {
              target,
              expiresAt: Date.now() + 1_200,
            };
            onSeek(target);
          }}
          minimumTrackTintColor="white"
          maximumTrackTintColor="#66666d"
          thumbTintColor="white"
          style={styles.slider}
        />
      </View>
      <View style={styles.times}>
        <Text style={styles.time}>{time(shownTime)}</Text>
        <Text style={styles.time}>
          -{time(Math.max(0, duration - shownTime))}
        </Text>
      </View>
    </>
  );
}

function QueuePanel({ player }: { player: Player }) {
  const sectionStart = player.radio.sectionStart;
  return (
    <View style={styles.panel}>
      {player.currentIndex < player.queue.length - 1 ? (
        <Pressable
          accessibilityRole="button"
          style={styles.clearQueue}
          onPress={player.clearUpcoming}
        >
          <Text style={styles.clearQueueText}>Clear upcoming</Text>
        </Pressable>
      ) : null}
      <FlatList
        data={player.queue}
        contentContainerStyle={{ paddingVertical: 10 }}
        keyExtractor={(item, index) => `${item.id}-${index}`}
        removeClippedSubviews
        ListFooterComponent={
          sectionStart < 0 ? <RadioQueueHeader hasPicks={false} /> : null
        }
        renderItem={({ item, index }) => {
          const radioPick = sectionStart >= 0 && index >= sectionStart;
          const row = (
            <SongRow
              song={item}
              queue={player.queue}
              index={index}
              onRemove={
                index === player.currentIndex
                  ? undefined
                  : () => player.removeFromQueue(index)
              }
              removeLabel="Remove from queue"
            />
          );
          return (
            <>
              {index === sectionStart ? <RadioQueueHeader hasPicks /> : null}
              {radioPick ? (
                <View style={styles.radioRow}>
                  <View style={{ flex: 1 }}>{row}</View>
                  <RadioThumbs index={index} name={item.name} size={18} />
                </View>
              ) : (
                row
              )}
            </>
          );
        }}
      />
    </View>
  );
}

function LyricsPanel({
  artworkPath,
  pending,
  synced,
  plainLyrics,
  instrumental,
  currentTime,
  onSeek,
  failure,
  onRetry,
  offline,
  source,
}: {
  artworkPath?: string | null;
  pending: boolean;
  synced: SyncedLine[];
  plainLyrics?: string | null;
  instrumental?: boolean;
  currentTime: number;
  onSeek: (seconds: number) => void;
  failure: LyricsFailure | null;
  onRetry: () => void;
  offline: boolean;
  source: LyricsSourceKind | null;
}) {
  const artworkUri = imageUrl(artworkPath);
  const scrollRef = useRef<ScrollView>(null);
  const lineLayouts = useRef(new Map<number, { height: number; y: number }>());
  const lastCenteredLine = useRef(-1);
  const [viewportHeight, setViewportHeight] = useState(0);
  const [contentHeight, setContentHeight] = useState(0);
  const activeLine = activeLyricIndex(synced, currentTime);
  const hasTimed = synced.some((line) => line.text.trim());
  useEffect(() => {
    lineLayouts.current.clear();
    lastCenteredLine.current = -1;
  }, [synced]);
  useEffect(() => {
    const layout = lineLayouts.current.get(activeLine);
    if (!layout || !viewportHeight) return;
    const frame = requestAnimationFrame(() => {
      // The sung line sits a little above the middle.
      scrollRef.current?.scrollTo({
        y: Math.max(0, layout.y - (viewportHeight * 0.38 - layout.height / 2)),
        animated: lastCenteredLine.current >= 0,
      });
      lastCenteredLine.current = activeLine;
    });
    return () => cancelAnimationFrame(frame);
  }, [activeLine, contentHeight, viewportHeight]);
  const where = lyricsSourceLabel(source);
  const empty = lyricsEmptyMessage(failure, Boolean(instrumental));
  return (
    <View style={[styles.panel, styles.lyricsPanel]}>
      {artworkUri ? (
        <>
          <Image
            blurRadius={38}
            contentFit="cover"
            pointerEvents="none"
            source={{
              uri: artworkUri,
              headers: imageRequestHeaders(artworkUri),
            }}
            style={styles.lyricsBackdrop}
          />
          <View pointerEvents="none" style={styles.lyricsScrim} />
        </>
      ) : null}
      {!pending && !offline && (hasTimed || plainLyrics) ? (
        <View style={styles.lyricsSourceRow}>
          <View
            style={[
              styles.lyricsSourceChip,
              hasTimed ? styles.lyricsSourceTimed : null,
            ]}
          >
            <View
              style={[
                styles.lyricsSourceDot,
                hasTimed ? styles.lyricsSourceDotTimed : null,
              ]}
            />
            <Text
              numberOfLines={1}
              style={[
                styles.lyricsSourceText,
                hasTimed ? styles.lyricsSourceTextTimed : null,
              ]}
            >
              {hasTimed ? "Timed lyrics" : "Untimed lyrics"}
              {where ? ` · ${where}` : ""}
            </Text>
          </View>
        </View>
      ) : null}
      <ScrollView
        ref={scrollRef}
        onLayout={({ nativeEvent }) =>
          setViewportHeight(nativeEvent.layout.height)
        }
        onContentSizeChange={(_width, height) => setContentHeight(height)}
        contentContainerStyle={[
          styles.lyricsContent,
          hasTimed
            ? {
                paddingTop: Math.max(24, viewportHeight * 0.3),
                paddingBottom: Math.max(24, viewportHeight / 2),
              }
            : null,
        ]}
        showsVerticalScrollIndicator={false}
      >
        {offline ? (
          <View style={styles.emptyLyrics}>
            <Text style={styles.emptyTitle}>
              Lyrics are unavailable offline
            </Text>
            <Text style={styles.emptyDetail}>
              Reconnect to the server to read this track&apos;s lyrics.
            </Text>
          </View>
        ) : pending ? (
          <View accessibilityRole="progressbar" style={styles.loadingLyricsBox}>
            <View style={styles.loadingBars}>
              {[12, 18, 10, 16].map((height, bar) => (
                <View key={bar} style={[styles.loadingBar, { height }]} />
              ))}
            </View>
            <Text style={styles.loadingLyrics}>Finding lyrics…</Text>
            <Text style={styles.loadingDetail}>
              Reading the file&apos;s tags and any .lrc beside it, then LRCLIB
              when online lookups are on.
            </Text>
          </View>
        ) : hasTimed ? (
          synced.map((line, index) => {
            const captureLayout = ({
              nativeEvent,
            }: {
              nativeEvent: { layout: { height: number; y: number } };
            }) => lineLayouts.current.set(index, nativeEvent.layout);
            const emphasis = lyricLineEmphasis(index, activeLine);
            return line.text.trim() ? (
              <Text
                accessibilityState={{ selected: emphasis.state === "active" }}
                key={`${line.time}-${index}`}
                onPress={() => onSeek(line.time)}
                onLayout={captureLayout}
                style={[
                  styles.lyricLine,
                  { opacity: emphasis.opacity },
                  emphasis.state === "active" ? styles.activeLyric : null,
                ]}
              >
                {line.text}
              </Text>
            ) : (
              <View
                key={`${line.time}-${index}`}
                onLayout={captureLayout}
                style={styles.lyricSpacer}
              >
                {emphasis.state === "active" ? (
                  <Text style={styles.lyricBreak}>• • •</Text>
                ) : null}
              </View>
            );
          })
        ) : plainLyrics ? (
          <Text style={styles.lyrics}>{plainLyrics}</Text>
        ) : (
          <Pressable
            accessibilityRole={empty.retry ? "button" : undefined}
            disabled={!empty.retry}
            onPress={onRetry}
            style={styles.emptyLyrics}
          >
            <Text style={styles.emptyTitle}>{empty.title}</Text>
            <Text style={styles.emptyDetail}>{empty.detail}</Text>
            {empty.retry ? (
              <Text style={styles.retryText}>Tap to try again.</Text>
            ) : null}
          </Pressable>
        )}
      </ScrollView>
    </View>
  );
}

function PositionedLyricsPanel(
  props: Omit<Parameters<typeof LyricsPanel>[0], "currentTime">,
) {
  const { currentTime } = usePlayerPosition();
  return <LyricsPanel {...props} currentTime={currentTime} />;
}

function PlayerDrawers({
  player,
  song,
  router,
  actionsOpen,
  onActionsClose,
  soundOpen,
  onSoundClose,
  online,
}: {
  player: Player;
  song: PlayerSong;
  router: Router;
  actionsOpen: boolean;
  onActionsClose: () => void;
  soundOpen: boolean;
  onSoundClose: () => void;
  online: boolean;
}) {
  const playback = useZonePlayback();
  const zones = useZones();
  const [playlistPicker, setPlaylistPicker] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const [downloadError, setDownloadError] = useState(false);
  return (
    <>
      <ActionDrawer
        open={actionsOpen}
        onClose={onActionsClose}
        title={song.name}
      >
        <DrawerAction
          icon={ListPlus}
          label="Play next"
          onPress={() => {
            playback.addNext(song);
            onActionsClose();
          }}
        />
        <DrawerAction
          icon={ListEnd}
          label="Add to queue"
          onPress={() => {
            playback.addToQueue([song]);
            onActionsClose();
          }}
        />
        {online ? (
          <StartRadioAction
            seed={{ kind: "song", id: song.id }}
            onStarted={onActionsClose}
          />
        ) : null}
        <BanAction
          target={{ kind: "song", id: song.id, name: song.name }}
          onDone={onActionsClose}
        />
        {online && song.album_object?.id ? (
          <DrawerAction
            icon={Disc3}
            label="View album"
            onPress={() => {
              onActionsClose();
              router.replace(`/album/${song.album_object.id}`);
            }}
          />
        ) : null}
        {online && song.artist_object?.id ? (
          <DrawerAction
            icon={UserRound}
            label="View artist"
            onPress={() => {
              onActionsClose();
              router.replace(`/artist/${song.artist_object.id}`);
            }}
          />
        ) : null}
        {online ? (
          <DrawerAction
            icon={Tv}
            label="Display mode on this screen"
            onPress={() => {
              onActionsClose();
              const zoneId = zones.selectedZone?.id ?? zones.localZoneId;
              router.push(
                `/display?zone=${encodeURIComponent(zoneId ?? "follow")}`,
              );
            }}
          />
        ) : null}
        {online ? (
          <DrawerAction
            icon={ListPlus}
            label="Add to playlist"
            onPress={() => {
              onActionsClose();
              setPlaylistPicker(true);
            }}
          />
        ) : null}
        {online || isSongDownloaded(song.id) ? (
          <DrawerAction
            icon={isSongDownloaded(song.id) ? X : Download}
            label={
              isSongDownloaded(song.id)
                ? "Delete from device"
                : downloading
                  ? "Downloading song…"
                  : downloadError
                    ? "Download failed · Try again"
                    : "Download song"
            }
            onPress={() => {
              if (downloading) return;
              if (isSongDownloaded(song.id)) {
                void removeDownload(song.id)
                  .then(onActionsClose)
                  .catch(() => setDownloadError(true));
                return;
              }
              setDownloadError(false);
              setDownloading(true);
              void downloadSong(song)
                .then(onActionsClose)
                .catch(() => setDownloadError(true))
                .finally(() => setDownloading(false));
            }}
          />
        ) : null}
        {downloadError ? (
          <Text accessibilityRole="alert" style={styles.drawerError}>
            The download action failed. Please try again.
          </Text>
        ) : null}
      </ActionDrawer>
      {player.audioPresetsEnabled ? (
        <ActionDrawer open={soundOpen} onClose={onSoundClose} title="Sound">
          {playerAudioPresets.map((preset) => (
            <DrawerAction
              key={preset.id}
              icon={presetIcons[preset.id]}
              label={`${player.audioPreset === preset.id ? "✓  " : ""}${preset.label}`}
              onPress={() => {
                player.setAudioPreset(preset.id);
                onSoundClose();
              }}
            />
          ))}
        </ActionDrawer>
      ) : null}
      <PlaylistPicker
        open={online && playlistPicker}
        onClose={() => setPlaylistPicker(false)}
        songId={song.id}
      />
    </>
  );
}

export default function PlayerScreen() {
  const router = useRouter();
  const client = useQueryClient();
  const session = useSession();
  const player = usePlayer();
  // "View lyrics" in a track menu opens the player on its lyrics.
  const params = useLocalSearchParams<{ panel?: string }>();
  const [panel, setPanel] = useState<Panel>(
    params.panel === "lyrics" || params.panel === "queue"
      ? params.panel
      : "player",
  );
  const [actions, setActions] = useState(false);
  const [soundOptions, setSoundOptions] = useState(false);
  const [signalPathOpen, setSignalPathOpen] = useState(false);
  const signalPath = useSignalPath(player.current);
  const [favoriteErrorSong, setFavoriteErrorSong] = useState<string | null>(
    null,
  );
  const { height: screenHeight } = useWindowDimensions();
  const translateY = useSharedValue(0);
  const dismissPlayer = useCallback(() => router.back(), [router]);
  const playerDragStyle = useAnimatedStyle(() => ({
    transform: [{ translateY: translateY.value }],
  }));
  const dragDownGesture = useMemo(
    () =>
      Gesture.Pan()
        .enabled(panel === "player")
        .activeOffsetY(14)
        .failOffsetX([-28, 28])
        .onUpdate(({ translationY }) => {
          translateY.value = Math.max(0, translationY);
        })
        .onEnd(({ translationY, velocityY }) => {
          if (translationY > 72 || velocityY > 700) {
            translateY.value = withTiming(
              screenHeight,
              { duration: 170 },
              () => {
                runOnJS(dismissPlayer)();
              },
            );
          } else {
            translateY.value = withSpring(0, { damping: 22, stiffness: 260 });
          }
        }),
    [dismissPlayer, panel, screenHeight, translateY],
  );
  useDownloadsRevision();
  const lyrics = useQuery({
    queryKey: ["lyrics", player.current?.id],
    queryFn: () => findLyrics(player.current!.id),
    enabled:
      panel === "lyrics" && !!player.current && session.phase === "ready",
    staleTime: 30 * 60_000,
    retry: false,
  });
  const bans = useRadioBans();
  const favorite = useQuery({
    queryKey: ["favorite-membership", player.current?.id],
    queryFn: () => isFavoriteSong(player.current!.id),
    enabled: !!player.current && session.phase === "ready",
    staleTime: 30_000,
  });
  const synced = useMemo(
    () => parseLrc(lyrics.data?.syncedLyrics),
    [lyrics.data?.syncedLyrics],
  );
  if (!player.current)
    return (
      <View style={styles.page}>
        <SafeAreaView>
          <Pressable
            {...immediateBorderlessPressFeedback}
            accessibilityLabel="Close player"
            accessibilityRole="button"
            style={styles.down}
            onPress={router.back}
          >
            <ChevronDown color="white" />
          </Pressable>
        </SafeAreaView>
      </View>
    );
  const song = player.current;
  const backdropUri = isLiveRadioId(song.id)
    ? null
    : imageUrl(song.album_object?.cover_url);
  const songBanned = bans.isBanned({ kind: "song", id: song.id });
  /** Heart cycle: not liked, liked, banned, then back to not liked. */
  const toggleFavorite = async () => {
    if (session.phase !== "ready") return;
    setFavoriteErrorSong(null);
    const state = heartState(!!favorite.data, songBanned);
    const target = { kind: "song" as const, id: song.id, name: song.name };
    if (state === "banned") {
      if (!(await bans.setBanned(target, false))) {
        setFavoriteErrorSong(song.id);
      }
      return;
    }
    if (state === "liked" && !(await bans.setBanned(target, true))) {
      setFavoriteErrorSong(song.id);
      return;
    }
    const next = !favorite.data;
    client.setQueryData(["favorite-membership", song.id], next);
    try {
      if (next) await addFavoriteSong(song.id);
      else await removeFavoriteSong(song.id);
      await client.invalidateQueries({ queryKey: ["favorite-song-details"] });
    } catch {
      client.setQueryData(["favorite-membership", song.id], !next);
      setFavoriteErrorSong(song.id);
    }
  };
  return (
    <GestureDetector gesture={dragDownGesture}>
      <AnimatedLinearGradient
        colors={["#26262B", "#0A0A0B", "#050506"]}
        style={[styles.page, playerDragStyle]}
      >
        {backdropUri ? (
          <>
            <Image
              blurRadius={60}
              contentFit="cover"
              pointerEvents="none"
              source={{
                uri: backdropUri,
                headers: imageRequestHeaders(backdropUri),
              }}
              style={styles.backdrop}
            />
            <LinearGradient
              colors={[
                "rgba(5,5,6,0.25)",
                "rgba(5,5,6,0.7)",
                "rgba(5,5,6,0.96)",
              ]}
              pointerEvents="none"
              style={StyleSheet.absoluteFill}
            />
          </>
        ) : null}
        <SafeAreaView edges={["top", "bottom"]} style={{ flex: 1 }}>
          <PlayerHeader
            panel={panel}
            onBack={() =>
              panel === "player" ? router.back() : setPanel("player")
            }
            onActions={() => setActions(true)}
          />
          {panel === "player" ? (
            <NowPlayingPanel
              player={player}
              song={song}
              router={router}
              favorite={!!favorite.data}
              banned={songBanned}
              onToggleFavorite={() => void toggleFavorite()}
              onQueue={() => setPanel("queue")}
              onLyrics={() => setPanel("lyrics")}
              onSound={() => setSoundOptions(true)}
              onSignalPath={() => setSignalPathOpen(true)}
              signalPath={signalPath.path}
              notice={
                player.error ??
                (favoriteErrorSong === song.id
                  ? "Could not update liked songs."
                  : null)
              }
              online={session.phase === "ready"}
            />
          ) : panel === "queue" ? (
            <QueuePanel player={player} />
          ) : (
            <PositionedLyricsPanel
              artworkPath={song.album_object?.cover_url}
              pending={lyrics.isPending}
              synced={synced}
              plainLyrics={lyrics.data?.plainLyrics}
              instrumental={lyrics.data?.instrumental}
              onSeek={player.seek}
              failure={lyrics.isError ? lyricsFailureKind(lyrics.error) : null}
              onRetry={() => void lyrics.refetch()}
              offline={session.phase === "offline"}
              source={(lyrics.data?.source ?? null) as LyricsSourceKind | null}
            />
          )}
          <PlayerDrawers
            player={player}
            song={song}
            router={router}
            actionsOpen={actions}
            onActionsClose={() => setActions(false)}
            soundOpen={soundOptions}
            onSoundClose={() => setSoundOptions(false)}
            online={session.phase === "ready"}
          />
          <ActionDrawer
            open={signalPathOpen}
            onClose={() => setSignalPathOpen(false)}
          >
            <SignalPathContent
              casting={player.signal.casting}
              levelingMode={player.volumeLevelingMode}
              loading={signalPath.loading}
              onLevelingModeChange={player.setVolumeLevelingMode}
              path={signalPath.path}
              dsp={dspAvailable(player.signal.platform) ? player.dsp : null}
              onDspChange={player.setDsp}
            />
            {session.phase === "ready" && !player.signal.casting ? (
              <PlayerServerOutputs />
            ) : null}
          </ActionDrawer>
        </SafeAreaView>
      </AnimatedLinearGradient>
    </GestureDetector>
  );
}

/** Moves the queue to one of the zones the server plays itself. */
function PlayerServerOutputs() {
  const player = usePlayer();
  return (
    <ServerOutputs
      onHandOff={() => {
        if (player.isPlaying) player.toggle();
      }}
    />
  );
}

/** A live station's current title (from its stream), polled. */
function LiveTitle({ songId, fallback }: { songId: string; fallback: string }) {
  const station = liveStationIdFromSongId(songId);
  const playing = useQuery({
    queryKey: ["live-radio", "now-playing", station],
    queryFn: async () =>
      (await getLiveNowPlaying([station!]))[station!] ?? null,
    enabled: Boolean(station),
    refetchInterval: LIVE_TITLE_POLL_MS,
  }).data;
  return <>{liveTitleText(playing?.title) || fallback}</>;
}

const styles = StyleSheet.create({
  liveRow: { alignItems: "center", flexDirection: "row", gap: 8, marginTop: 6 },
  liveBar: { height: 44, justifyContent: "center", paddingHorizontal: 4 },
  liveLine: {
    backgroundColor: "#f2473f",
    borderRadius: 2,
    height: 3,
    opacity: 0.85,
  },
  page: { flex: 1, backgroundColor: "black" },
  down: { padding: 20 },
  header: {
    height: 56,
    paddingHorizontal: 20,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  panelHeader: { color: "white", fontSize: 15, fontWeight: "800" },
  playbackError: {
    color: "#f6807a",
    fontSize: 13,
    textAlign: "center",
  },
  disabledControl: { opacity: 0.4 },
  retryText: { color: palette.secondary, fontSize: 14, marginTop: 8 },
  player: { flex: 1, paddingHorizontal: 26, justifyContent: "space-evenly" },
  art: { alignSelf: "center", maxWidth: "100%", aspectRatio: 1 },
  songInfo: {
    marginTop: 14,
    flexDirection: "row",
    alignItems: "center",
    gap: 14,
  },
  title: {
    color: palette.text,
    fontFamily: fonts.display,
    fontSize: 30,
    lineHeight: 36,
  },
  meta: { color: palette.tertiary, fontSize: 12.5, marginTop: 6 },
  backdrop: {
    position: "absolute",
    top: 0,
    right: 0,
    bottom: 0,
    left: 0,
    opacity: 0.5,
  },
  artist: { color: palette.secondary, fontSize: 16, marginTop: 5 },
  sliderHitbox: {
    width: "100%",
    height: 52,
    marginTop: 2,
    justifyContent: "center",
  },
  slider: { width: "100%", height: 52 },
  times: {
    flexDirection: "row",
    justifyContent: "space-between",
    marginTop: -12,
  },
  time: {
    color: palette.secondary,
    fontSize: 11,
    fontVariant: ["tabular-nums"],
  },
  controls: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    marginVertical: 10,
  },
  playButton: {
    width: 68,
    height: 68,
    borderRadius: 34,
    backgroundColor: palette.accent,
    alignItems: "center",
    justifyContent: "center",
  },
  panelButtons: { flexDirection: "row", gap: 8 },
  upNext: {
    alignItems: "center",
    backgroundColor: "rgba(0,0,0,0.25)",
    borderColor: palette.border,
    borderRadius: 10,
    borderWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    gap: 12,
    padding: 8,
  },
  upNextLabel: {
    color: palette.tertiary,
    fontSize: 10,
    fontWeight: "600",
    letterSpacing: 1.4,
  },
  upNextTitle: { color: palette.text, fontSize: 14, marginTop: 2 },
  upNextArtist: { color: palette.tertiary },
  castButton: {
    width: 46,
    height: 46,
    alignItems: "center",
    justifyContent: "center",
  },
  panelButton: {
    flex: 1,
    height: 46,
    borderRadius: 12,
    backgroundColor: "rgba(255,255,255,.1)",
    alignItems: "center",
    justifyContent: "center",
  },
  panelText: { color: "white", fontWeight: "800" },
  panel: { flex: 1, paddingTop: 18 },
  radioLabel: {
    color: "#a9d1e6",
    fontSize: 11,
    fontWeight: "800",
    letterSpacing: 1.2,
    marginBottom: 4,
    textTransform: "uppercase",
  },
  feedbackRow: { flexDirection: "row", marginTop: -4, marginLeft: -8 },
  radioRow: { flexDirection: "row", alignItems: "center", paddingRight: 12 },
  lyricsPanel: {
    overflow: "hidden",
    backgroundColor: "#111114",
  },
  lyricsBackdrop: {
    position: "absolute",
    inset: 0,
    opacity: 0.58,
    transform: [{ scale: 1.16 }],
  },
  lyricsScrim: {
    position: "absolute",
    inset: 0,
    backgroundColor: "rgba(5,5,6,.62)",
  },
  clearQueue: {
    alignSelf: "flex-end",
    marginHorizontal: 20,
    minHeight: 44,
    justifyContent: "center",
  },
  clearQueueText: { color: palette.secondary, fontWeight: "700" },
  panelTitle: {
    color: "white",
    fontSize: 30,
    fontWeight: "900",
    paddingHorizontal: 20,
  },
  lyrics: {
    color: "white",
    fontSize: 24,
    lineHeight: 35,
    fontWeight: "700",
    paddingHorizontal: 24,
    textAlign: "center",
  },
  lyricLine: {
    color: palette.text,
    fontFamily: fonts.titleMedium,
    fontSize: 27,
    lineHeight: 34,
    paddingHorizontal: 24,
    paddingVertical: 7,
  },
  lyricBreak: {
    color: palette.text,
    fontSize: 22,
    letterSpacing: 4,
    paddingHorizontal: 24,
  },
  lyricsSourceRow: {
    paddingHorizontal: 20,
    paddingTop: 8,
    flexDirection: "row",
  },
  lyricsSourceChip: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    maxWidth: "100%",
    borderWidth: 1,
    borderColor: "rgba(242,238,230,0.14)",
    borderRadius: 999,
    paddingHorizontal: 10,
    paddingVertical: 4,
    backgroundColor: "rgba(0,0,0,.25)",
  },
  lyricsSourceTimed: {
    borderColor: "rgba(217,139,95,0.4)",
    backgroundColor: "rgba(217,139,95,0.1)",
  },
  lyricsSourceDot: {
    width: 6,
    height: 6,
    borderRadius: 3,
    backgroundColor: palette.tertiary,
  },
  lyricsSourceDotTimed: { backgroundColor: palette.accent },
  lyricsSourceText: { color: palette.secondary, fontSize: 11, flexShrink: 1 },
  lyricsSourceTextTimed: { color: palette.accent },
  loadingLyricsBox: {
    alignItems: "center",
    paddingHorizontal: 32,
    marginTop: 80,
  },
  loadingBars: {
    flexDirection: "row",
    alignItems: "flex-end",
    gap: 3,
    height: 20,
  },
  loadingBar: { width: 3, borderRadius: 2, backgroundColor: palette.accent },
  loadingDetail: {
    color: palette.tertiary,
    fontSize: 13,
    lineHeight: 19,
    marginTop: 8,
    textAlign: "center",
  },
  lyricsContent: {
    flexGrow: 1,
    paddingVertical: 24,
  },
  activeLyric: { color: palette.text },
  lyricSpacer: { minHeight: 20, justifyContent: "center" },
  loadingLyrics: {
    color: palette.text,
    fontFamily: fonts.title,
    fontSize: 22,
    textAlign: "center",
    marginTop: 16,
  },
  emptyLyrics: {
    padding: 24,
    margin: 20,
    backgroundColor: "rgba(255,255,255,.08)",
    borderRadius: 16,
  },
  emptyTitle: { color: "white", fontWeight: "900", fontSize: 18 },
  emptyDetail: { color: palette.secondary, lineHeight: 20, marginTop: 8 },
  drawerError: { color: "#f6807a", paddingHorizontal: 16, paddingVertical: 8 },
});
