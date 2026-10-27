import { Suspense } from "react";
import type { Metadata } from "next";
import RouteLoading from "@/components/app/route-loading";
import StationDetails from "@/features/live-radio/station-details";

export const metadata: Metadata = {
  title: "Station",
  description: "A live radio station, what it is playing and how it sounds.",
};

export default function StationPage() {
  return (
    <Suspense fallback={<RouteLoading />}>
      <StationDetails />
    </Suspense>
  );
}
