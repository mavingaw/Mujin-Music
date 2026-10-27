const FAILURE_COPY = Object.freeze({
  database_migration_failed: {
    title: "Database update failed",
    body: "Mujin Music couldn’t update its library database. Your music files were not changed. Open the logs before trying again.",
    action: "Open logs",
  },
  host_unavailable: {
    title: "Local Mujin Music host unavailable",
    body: "The local music service did not start. Check the technical details, then try again.",
    action: "Try again",
  },
  webview_failed: {
    title: "Mujin Music’s window could not load",
    body: "The app window failed to load the local Mujin Music interface. Restart the view; if it happens again, open the logs.",
    action: "Try again",
  },
});

function classifyStartupFailure(detail) {
  const normalized = String(detail || "").toLowerCase();
  return normalized.includes("migration") ||
    normalized.includes("database update") ||
    normalized.includes("schema")
    ? "database_migration_failed"
    : "host_unavailable";
}

function failureCopy(kind) {
  return FAILURE_COPY[kind] || FAILURE_COPY.host_unavailable;
}

module.exports = { FAILURE_COPY, classifyStartupFailure, failureCopy };
