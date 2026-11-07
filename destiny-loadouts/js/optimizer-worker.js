// Runs the optimizer off the main thread so the page stays responsive.
import { optimize } from "./optimizer.js";

self.onmessage = (event) => {
  // A dedicated worker only hears from the page that started it, where the
  // origin is empty; anything that names a different origin is ignored.
  if (event.origin && event.origin !== self.location.origin) return;
  const { piecesBySlot, options } = event.data;
  try {
    self.postMessage({ ok: true, result: optimize(piecesBySlot, options) });
  } catch (error) {
    self.postMessage({ ok: false, error: String(error?.message ?? error) });
  }
};
