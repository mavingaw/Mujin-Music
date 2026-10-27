import { Suspense } from "react";
import type { Metadata } from "next";
import RouteLoading from "@/components/app/route-loading";
import StreamingPlaylistPage from "@/features/streaming/streaming-playlists";

export const metadata: Metadata = {
  title: "Playlist",
  description:
    "A TIDAL or Qobuz playlist, read-only, or its copy kept and edited in Mujin Music.",
};

export default function StreamingPlaylistRoute() {
  return (
    <Suspense fallback={<RouteLoading />}>
      <StreamingPlaylistPage />
    </Suspense>
  );
}
