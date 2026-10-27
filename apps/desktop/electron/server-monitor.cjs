// Watches the server the window is showing, so the app can say clearly when
// it goes away, reconnect on its own when it comes back, and notice when the
// address now belongs to a different Parson library.
const { reconnectDelay } = require("./server-config.cjs");

function createServerMonitor({
  probe,
  instanceId = "",
  bootId = "",
  intervalMs = 4_000,
  failuresBeforeDown = 2,
  initiallyDown = false,
  onDown = () => {},
  onUp = () => {},
  onRestart = () => {},
  onReplaced = () => {},
  onAttempt = () => {},
  schedule = setTimeout,
  cancel = clearTimeout,
  now = Date.now,
}) {
  let state = initiallyDown ? "down" : "up";
  let failures = 0;
  let attempt = 0;
  let lastBootId = bootId;
  let lastSeenAt = now();
  let timer = null;
  let running = false;
  let inFlight = null;

  const plan = (delay) => {
    if (!running) return;
    if (timer) cancel(timer);
    timer = schedule(() => {
      timer = null;
      void tick();
    }, delay);
    timer?.unref?.();
  };

  async function tick() {
    if (inFlight) return inFlight;
    inFlight = (async () => {
      const result = await probe();
      if (!running) return state;
      if (result.kind === "ok") {
        const manifest = result.manifest ?? {};
        if (instanceId && manifest.instanceId !== instanceId) {
          state = "replaced";
          running = false;
          onReplaced(manifest);
          return state;
        }
        const restarted = Boolean(
          lastBootId && manifest.bootId && manifest.bootId !== lastBootId,
        );
        if (manifest.bootId) lastBootId = manifest.bootId;
        lastSeenAt = now();
        failures = 0;
        if (state === "down") {
          state = "up";
          attempt = 0;
          onUp({ restarted });
        } else if (restarted) {
          onRestart(manifest);
        }
        plan(intervalMs);
        return state;
      }
      failures += 1;
      if (state === "up" && failures >= failuresBeforeDown) {
        state = "down";
        attempt = 0;
        onDown({ lastSeenAt, reason: result.kind, message: result.message });
      }
      if (state === "down") {
        onAttempt({ attempt, nextInMs: reconnectDelay(attempt) });
        plan(reconnectDelay(attempt));
        attempt += 1;
      } else {
        // One missed probe: look again sooner than usual before declaring
        // the server gone, so a blip never flashes the reconnect screen.
        plan(Math.min(intervalMs, 1_000));
      }
      return state;
    })();
    try {
      return await inFlight;
    } finally {
      inFlight = null;
    }
  }

  return {
    start() {
      if (running) return;
      running = true;
      plan(state === "down" ? reconnectDelay(0) : intervalMs);
    },
    stop() {
      running = false;
      if (timer) cancel(timer);
      timer = null;
    },
    /** Probe now (e.g. after the computer wakes or the user taps Retry). */
    async check() {
      if (!running) return state;
      return tick();
    },
    state: () => state,
    lastSeenAt: () => lastSeenAt,
  };
}

module.exports = { createServerMonitor };
