import RouteLoading from "@/components/app/route-loading";
import LibraryToolsPage from "@/features/library/tools/library-tools-page";
import type { Metadata } from "next";
import { Suspense } from "react";

export const metadata: Metadata = {
  title: "Library tools",
  description:
    "Skipped files, duplicates, album versions, the Albumizer and library maintenance.",
};

export default function LibraryToolsRoute() {
  return (
    <Suspense fallback={<RouteLoading />}>
      <LibraryToolsPage />
    </Suspense>
  );
}
