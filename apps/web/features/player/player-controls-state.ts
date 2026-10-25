export function formatPlaybackTime(time: number) {
  if (!Number.isFinite(time) || time <= 0) return "0:00";
  const minutes = Math.floor(time / 60);
  const seconds = Math.floor(time % 60);
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

export function timelineValueText(currentTime: number, duration: number) {
  return `${formatPlaybackTime(currentTime)} of ${formatPlaybackTime(duration)}`;
}

/**
 * The speed the timeline reads times at. Audio presets (slowed, sped up)
 * only change this device's own playback; a remote zone or a cast session
 * plays at normal speed, so its times must not be scaled by them.
 */
export function timelinePlaybackRate(localRate: number, remote: boolean) {
  if (remote) return 1;
  return Number.isFinite(localRate) && localRate > 0 ? localRate : 1;
}
