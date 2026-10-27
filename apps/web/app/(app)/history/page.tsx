import { Suspense } from "react";
import type { Metadata } from "next";
import RouteLoading from "@/components/app/route-loading";
import HistoryPage from "@/features/library/history/history-page";

export const metadata: Metadata = {
  title: "History",
  description: "Your listening history, statistics and export.",
};

export default function HistoryRoute() {
  return (
    <Suspense fallback={<RouteLoading />}>
      <HistoryPage />
    </Suspense>
  );
}
