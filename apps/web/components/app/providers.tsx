"use client";

import { ReactNode, Suspense, useEffect, useState } from "react";
import { PlayerProvider } from "@/features/player/player-context";
import { ZonesProvider } from "@/features/player/zones/zones-provider";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import TitleMetadata from "@/components/app/title-metadata";
import { getLibraryUnavailable, isApiError } from "@parson/music-sdk";
import { watchDesktopServer } from "@/lib/desktop/bridge";

interface AppProvidersProps {
  children: ReactNode;
}

export default function AppProviders({ children }: AppProvidersProps) {
  const [queryClient] = useState(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: {
            retry: (failureCount, error) => {
              if (failureCount >= 2) return false;
              if (getLibraryUnavailable(error)) return false;
              if (!isApiError(error) || !error.response) return true;
              const status = error.response.status;
              return status === 408 || status === 429 || status >= 500;
            },
            retryDelay: (attempt) => Math.min(750 * 2 ** attempt, 5000),
            refetchOnWindowFocus: false,
            staleTime: 1000 * 60,
            gcTime: 1000 * 60 * 30,
          },
        },
      }),
  );

  useEffect(
    () =>
      // Parson Desktop noticed the server restarted (an update, a reboot of
      // the NAS): refetch everything so no view keeps stale state.
      watchDesktopServer((event) => {
        if (event.type === "restarted") void queryClient.invalidateQueries();
      }),
    [queryClient],
  );

  return (
    <QueryClientProvider client={queryClient}>
      <PlayerProvider>
        <ZonesProvider>
          <Suspense>
            <TitleMetadata />
          </Suspense>
          {children}
        </ZonesProvider>
      </PlayerProvider>
    </QueryClientProvider>
  );
}
