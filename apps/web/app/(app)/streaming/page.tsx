import { Suspense } from "react";
import type { Metadata } from "next";
import RouteLoading from "@/components/app/route-loading";
import StreamingBrowse from "@/features/streaming/streaming-browse";

export const metadata: Metadata = {
  title: "Streaming",
  description: "Browse TIDAL and Qobuz.",
};

export default function StreamingPage() {
  return (
    <Suspense fallback={<RouteLoading />}>
      <StreamingBrowse />
    </Suspense>
  );
}
