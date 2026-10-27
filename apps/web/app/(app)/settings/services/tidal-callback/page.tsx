import { Suspense } from "react";
import type { Metadata } from "next";
import RouteLoading from "@/components/app/route-loading";
import TidalCallback from "@/features/streaming/tidal-callback";

export const metadata: Metadata = {
  title: "TIDAL sign-in",
  description:
    "Finishes signing in to TIDAL after approving Mujin Music on TIDAL.",
};

export default function TidalCallbackRoute() {
  return (
    <Suspense fallback={<RouteLoading />}>
      <TidalCallback />
    </Suspense>
  );
}
