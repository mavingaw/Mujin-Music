import { Suspense } from "react";
import type { Metadata } from "next";
import RouteLoading from "@/components/app/route-loading";
import ListenLaterPage from "@/features/library/listen-later/listen-later-page";

export const metadata: Metadata = {
  title: "Listen Later",
  description: "Albums, tracks and artists you saved for later.",
};

export default function ListenLaterRoute() {
  return (
    <Suspense fallback={<RouteLoading />}>
      <ListenLaterPage />
    </Suspense>
  );
}
