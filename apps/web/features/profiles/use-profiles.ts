"use client";

import {
  getProfiles,
  refreshMediaToken,
  switchProfile,
  type AuthResponse,
  type Profile,
  type SwitchProfileInput,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { toast } from "sonner";
import { useSession } from "@/features/account/session-provider";

export const profilesKey = ["profiles"] as const;

export function useProfiles(enabled = true) {
  const { session } = useSession();
  return useQuery({
    queryKey: [...profilesKey, session?.sub ?? "signed-out"],
    queryFn: getProfiles,
    enabled: enabled && Boolean(session),
    staleTime: 30_000,
  });
}

/**
 * Switches this device to another profile. The session, media token and
 * every cached query move to the new profile; playback keeps going.
 */
export function useProfileSwitch() {
  const queryClient = useQueryClient();
  const { setSession } = useSession();
  return useCallback(
    async (
      profile: Profile,
      input: SwitchProfileInput = {},
    ): Promise<AuthResponse> => {
      const result = await switchProfile(profile.id, input);
      if (!result.status || !result.claims) return result;
      await refreshMediaToken().catch(() => undefined);
      setSession(result.claims);
      await queryClient.resetQueries();
      window.dispatchEvent(
        new CustomEvent("parson:profile-changed", {
          detail: { id: profile.id },
        }),
      );
      toast.success(`Listening as ${profile.name}`, {
        description: "History, favourites and Radio now follow this profile.",
      });
      return result;
    },
    [queryClient, setSession],
  );
}
