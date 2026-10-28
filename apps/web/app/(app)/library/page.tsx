import LibraryOverview from "@/features/library/library-overview";
import type { Metadata } from "next";
import RouteLoading from "@/components/app/route-loading";
import { Suspense } from "react";

export const metadata: Metadata = {
  title: "Library",
  description: "Your music library.",
};

export default function LibraryPage() {
  return (
    <div className="px-5 pb-36 pt-8 sm:px-8 md:pt-12 lg:px-12">
      <Suspense fallback={<RouteLoading />}>
        <LibraryOverview />
      </Suspense>
    </div>
  );
}
