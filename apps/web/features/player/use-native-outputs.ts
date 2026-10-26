"use client";

import { getNativeOutputs, type NativeOutputs } from "@parson/music-sdk";
import { useQuery } from "@tanstack/react-query";
import { useSession } from "@/features/account/session-provider";

/** Output choices for server devices are stored with this prefix. */
export const SERVER_OUTPUT_PREFIX = "server:";

export const nativeOutputsKey = ["native-outputs"] as const;

/** The server device id in an output choice, or `null` for this browser. */
export function serverOutputId(choice: string): string | null {
  return choice.startsWith(SERVER_OUTPUT_PREFIX)
    ? choice.slice(SERVER_OUTPUT_PREFIX.length) || null
    : null;
}

/**
 * The server's own audio devices (bit-perfect outputs). While one is in
 * use the list is refreshed, so a DSP chain changed from another client
 * (a phone acting as remote) shows here too.
 */
export function useNativeOutputs(options: { live?: boolean } = {}) {
  const { session } = useSession();
  return useQuery<NativeOutputs>({
    queryKey: nativeOutputsKey,
    queryFn: getNativeOutputs,
    enabled: Boolean(session?.sub),
    staleTime: 30_000,
    refetchInterval: options.live ? 5_000 : false,
    retry: 1,
  });
}
