import { Suspense } from "react";
import type { Metadata } from "next";
import RouteLoading from "@/components/app/route-loading";
import TagsOverview from "@/features/library/tags/tags-overview";

export const metadata: Metadata = {
  title: "Tags",
  description: "Your tags and smart tags.",
};

export default function TagsRoute() {
  return (
    <Suspense fallback={<RouteLoading />}>
      <TagsOverview />
    </Suspense>
  );
}
