import { Suspense } from "react";
import type { Metadata } from "next";
import RouteLoading from "@/components/app/route-loading";
import LastfmCallback from "@/features/scrobbling/lastfm-callback";

export const metadata: Metadata = {
  title: "Connect Last.fm",
  description: "Finishes connecting a Parson profile to Last.fm.",
};

export default function LastfmCallbackRoute() {
  return (
    <Suspense fallback={<RouteLoading />}>
      <LastfmCallback />
    </Suspense>
  );
}
