// Runs the optimizer off the main thread so the page stays responsive.
import { optimize } from "./optimizer.js";

self.onmessage = (event) => {
  const { piecesBySlot, options } = event.data;
  try {
    self.postMessage({ ok: true, result: optimize(piecesBySlot, options) });
  } catch (error) {
    self.postMessage({ ok: false, error: String(error?.message ?? error) });
  }
};
