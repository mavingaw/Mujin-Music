import { Suspense } from "react";
import type { Metadata } from "next";
import RouteLoading from "@/components/app/route-loading";
import TagDetails from "@/features/library/tags/tag-details";

export const metadata: Metadata = {
  title: "Tag",
  description: "Everything carrying one tag.",
};

export default function TagRoute() {
  return (
    <Suspense fallback={<RouteLoading />}>
      <TagDetails />
    </Suspense>
  );
}
