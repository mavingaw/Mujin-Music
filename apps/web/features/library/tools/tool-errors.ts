import { isApiError } from "@parson/music-sdk";

/** The server's own message for a failed request, or `fallback`. */
export function getApiErrorMessage(error: unknown, fallback: string): string {
  if (isApiError(error)) {
    const data = error.response?.data as { message?: unknown } | undefined;
    if (typeof data?.message === "string" && data.message.trim()) {
      return data.message;
    }
  }
  return fallback;
}
