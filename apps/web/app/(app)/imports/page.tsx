import type { Metadata } from "next";
import { Suspense } from "react";
import RouteLoading from "@/components/app/route-loading";
import ImportsPage from "@/features/imports/imports-page";

export const metadata: Metadata = {
  title: "Imports",
  description:
    "Import playlists from Spotify, Apple Music, YouTube Music, Deezer and playlist files, matched to your own library.",
};

export default function Page() {
  return (
    <Suspense fallback={<RouteLoading />}>
      <ImportsPage />
    </Suspense>
  );
}
