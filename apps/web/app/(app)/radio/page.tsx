import { Suspense } from "react";
import type { Metadata } from "next";
import RouteLoading from "@/components/app/route-loading";
import LiveRadioBrowse from "@/features/live-radio/live-radio-browse";

export const metadata: Metadata = {
  title: "Live Radio",
  description:
    "Internet radio from around the world, by genre, country, language and quality.",
};

export default function LiveRadioPage() {
  return (
    <Suspense fallback={<RouteLoading />}>
      <LiveRadioBrowse />
    </Suspense>
  );
}
