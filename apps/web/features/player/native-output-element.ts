"use client";

import {
  getNativeOutputSession,
  playOnNativeOutput,
  sendNativeOutputCommand,
  type NativeOutputCommand,
  type NativeOutputLeveling,
  type NativeOutputSession,
} from "@parson/music-sdk";

/** Server calls the element makes; replaced in tests. */
export type NativeOutputTransport = {
  play: (input: {
    device_id: string;
    song_id: string;
    position_ms: number;
    leveling?: NativeOutputLeveling | null;
  }) => Promise<NativeOutputSession>;
  command: (
    command: NativeOutputCommand,
    positionMs?: number,
  ) => Promise<NativeOutputSession | null>;
  session: () => Promise<NativeOutputSession | null>;
};

const sdkTransport: NativeOutputTransport = {
  play: playOnNativeOutput,
  command: sendNativeOutputCommand,
  session: getNativeOutputSession,
};

/** `.../media/songs/{id}/stream?...` to the song id. */
export function songIdFromStreamUrl(url: string): string | null {
  const match = /\/media\/songs\/([^/?#]+)\/stream(?:[?#]|$)/.exec(url);
  if (!match?.[1]) return null;
  try {
    return decodeURIComponent(match[1]);
  } catch {
    return null;
  }
}

const MEDIA_ERR_NETWORK = 2;
const MEDIA_ERR_SRC_NOT_SUPPORTED = 4;

/**
 * Stands in for the `<audio>` element while a server output plays.
 *
 * The player keeps driving an element (src, play, pause, currentTime,
 * media events), so queueing, telemetry and the transport UI work
 * unchanged. Underneath, each call becomes a request to the server, which
 * decodes the file and plays it on its own device; the element polls the
 * session and replays its state as `timeupdate`, `pause`, `ended` and
 * `error` events. Nothing is downloaded or decoded in the browser.
 */
export class NativeOutputElement extends EventTarget {
  readonly deviceId: string;
  crossOrigin: string | null = "anonymous";
  preload = "auto";
  // The server output has fixed volume (bit-perfect); these are accepted
  // and ignored so the player can treat this like any element.
  volume = 1;
  muted = false;
  playbackRate = 1;
  defaultPlaybackRate = 1;
  preservesPitch = true;
  error: { code: number; message: string } | null = null;

  private levelingValue: NativeOutputLeveling | null = null;
  private url = "";
  private songId: string | null = null;
  private isPaused = true;
  private isEnded = false;
  private positionMs = 0;
  private positionAt = 0;
  private durationMs = Number.NaN;
  private active = false;
  private timer: ReturnType<typeof setInterval> | null = null;
  private disposed = false;
  private readonly transport: NativeOutputTransport;
  // Requests go out one at a time, in order: a "pause" for the previous
  // track must not overtake the request that starts the next one.
  private queue: Promise<unknown> = Promise.resolve();
  private readonly pollMs: number;
  private readonly now: () => number;
  private readonly onSession: (session: NativeOutputSession | null) => void;

  constructor(
    deviceId: string,
    options: {
      transport?: NativeOutputTransport;
      pollMs?: number;
      now?: () => number;
      onSession?: (session: NativeOutputSession | null) => void;
    } = {},
  ) {
    super();
    this.deviceId = deviceId;
    this.transport = options.transport ?? sdkTransport;
    this.pollMs = options.pollMs ?? 500;
    this.now = options.now ?? (() => performance.now());
    this.onSession = options.onSession ?? (() => {});
  }

  /**
   * Volume leveling for the server to apply. Changing it while a track
   * plays restarts the track on the server at the current position.
   */
  get leveling() {
    return this.levelingValue;
  }

  set leveling(next: NativeOutputLeveling | null) {
    const same =
      next?.source === this.levelingValue?.source &&
      Math.abs((next?.gain_db ?? 0) - (this.levelingValue?.gain_db ?? 0)) <
        0.05 &&
      Boolean(next) === Boolean(this.levelingValue);
    this.levelingValue = next;
    if (same || !this.active || this.isPaused || this.isEnded) return;
    this.positionMs = this.currentTime * 1000;
    this.positionAt = this.now();
    this.active = false;
    void this.play().catch(() => {});
  }

  private send<T>(request: () => Promise<T>): Promise<T> {
    const next = this.queue.then(request, request);
    this.queue = next.catch(() => {});
    return next;
  }

  get src() {
    return this.url;
  }

  set src(next: string) {
    if (next === this.url) return;
    const wasActive = this.active;
    this.url = next;
    this.songId = songIdFromStreamUrl(next);
    this.isPaused = true;
    this.isEnded = false;
    this.positionMs = 0;
    this.positionAt = this.now();
    this.durationMs = Number.NaN;
    this.error = null;
    this.active = false;
    this.stopPolling();
    // A new track replaces the old one on the device when it plays; until
    // then the old one must not keep sounding.
    if (wasActive)
      void this.send(() => this.transport.command("pause")).catch(() => {});
  }

  get paused() {
    return this.isPaused;
  }

  get ended() {
    return this.isEnded;
  }

  get duration() {
    return this.durationMs / 1000;
  }

  get currentTime() {
    const elapsed = this.isPaused ? 0 : this.now() - this.positionAt;
    const position = this.positionMs + Math.max(0, elapsed);
    return (
      (Number.isFinite(this.durationMs)
        ? Math.min(position, this.durationMs)
        : position) / 1000
    );
  }

  set currentTime(seconds: number) {
    if (!Number.isFinite(seconds)) return;
    const target = Math.max(0, Math.round(seconds * 1000));
    this.positionMs = target;
    this.positionAt = this.now();
    this.isEnded = false;
    this.dispatchEvent(new Event("seeking"));
    if (this.active) {
      void this.send(() => this.transport.command("seek", target))
        .then((session) => this.apply(session))
        .catch(() => {})
        .finally(() => this.dispatchEvent(new Event("seeked")));
    } else {
      this.dispatchEvent(new Event("seeked"));
    }
    this.dispatchEvent(new Event("timeupdate"));
  }

  removeAttribute(name: string) {
    if (name === "src") this.src = "";
  }

  load() {}

  async play(): Promise<void> {
    if (this.disposed) throw new DOMException("Output closed", "AbortError");
    const songId = this.songId;
    if (!songId) {
      throw new DOMException("No track to play", "NotSupportedError");
    }
    try {
      const resumable = this.active && !this.isEnded;
      const session = resumable
        ? await this.send(() => this.transport.command("resume"))
        : await this.send(() =>
            this.transport.play({
              device_id: this.deviceId,
              song_id: songId,
              position_ms: this.isEnded ? 0 : this.positionMs,
              leveling: this.levelingValue,
            }),
          );
      if (this.songId !== songId) return;
      this.active = true;
      this.isEnded = false;
      // The reply can predate the server acting on "resume", so it is
      // applied while still paused and only then marked playing.
      this.isPaused = true;
      this.apply(session);
      if (!this.active) return;
      this.isPaused = false;
      this.positionAt = this.now();
      this.dispatchEvent(new Event("play"));
      this.dispatchEvent(new Event("playing"));
      this.startPolling();
    } catch (cause) {
      if (this.songId !== songId) return;
      this.isPaused = true;
      this.error = {
        code: MEDIA_ERR_SRC_NOT_SUPPORTED,
        message: cause instanceof Error ? cause.message : String(cause),
      };
      this.dispatchEvent(new Event("error"));
      throw new DOMException(this.error.message, "NotSupportedError");
    }
  }

  pause() {
    if (this.isPaused) return;
    this.positionMs = this.currentTime * 1000;
    this.positionAt = this.now();
    this.isPaused = true;
    this.dispatchEvent(new Event("pause"));
    if (this.active) {
      void this.send(() => this.transport.command("pause"))
        .then((session) => this.apply(session))
        .catch(() => {});
    }
  }

  /** Stops the server output and releases its device. */
  dispose() {
    this.disposed = true;
    this.stopPolling();
    if (this.active)
      void this.send(() => this.transport.command("stop")).catch(() => {});
    this.active = false;
    this.onSession(null);
  }

  /** One poll of the server session; exposed for tests. */
  async poll() {
    try {
      this.apply(await this.send(() => this.transport.session()));
    } catch {
      if (this.isPaused) return;
      this.isPaused = true;
      this.error = { code: MEDIA_ERR_NETWORK, message: "Server unreachable" };
      this.dispatchEvent(new Event("error"));
    }
  }

  private startPolling() {
    if (this.timer || this.disposed) return;
    this.timer = setInterval(() => void this.poll(), this.pollMs);
  }

  private stopPolling() {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
  }

  private apply(session: NativeOutputSession | null) {
    if (this.disposed || !this.active) return;
    const ours =
      session &&
      session.song_id === this.songId &&
      session.device_id === this.deviceId;
    if (!ours) {
      // Stopped elsewhere, or another client took the device.
      this.onSession(null);
      this.active = false;
      this.stopPolling();
      if (!this.isPaused) {
        this.isPaused = true;
        this.dispatchEvent(new Event("pause"));
      }
      return;
    }
    this.onSession(session);
    const hadDuration = Number.isFinite(this.durationMs);
    if (session.duration_ms > 0) this.durationMs = session.duration_ms;
    this.positionMs = session.position_ms;
    this.positionAt = this.now();
    if (!hadDuration && Number.isFinite(this.durationMs))
      this.dispatchEvent(new Event("loadedmetadata"));
    switch (session.state) {
      case "ended":
        this.stopPolling();
        this.isPaused = true;
        if (!this.isEnded) {
          this.isEnded = true;
          if (Number.isFinite(this.durationMs))
            this.positionMs = this.durationMs;
          this.dispatchEvent(new Event("timeupdate"));
          this.dispatchEvent(new Event("ended"));
        }
        return;
      case "failed":
        this.stopPolling();
        this.isPaused = true;
        this.error = {
          code: MEDIA_ERR_SRC_NOT_SUPPORTED,
          message: session.error ?? "The server output failed",
        };
        this.dispatchEvent(new Event("error"));
        return;
      case "stopped":
        this.active = false;
        this.stopPolling();
        if (!this.isPaused) {
          this.isPaused = true;
          this.dispatchEvent(new Event("pause"));
        }
        return;
      case "paused":
        if (!this.isPaused) {
          this.isPaused = true;
          this.dispatchEvent(new Event("pause"));
        }
        break;
      default:
        break;
    }
    this.dispatchEvent(new Event("timeupdate"));
  }
}
