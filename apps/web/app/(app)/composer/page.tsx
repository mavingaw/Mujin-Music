import { Suspense } from "react";
import type { Metadata } from "next";
import RouteLoading from "@/components/app/route-loading";
import ComposerDetails from "./composer-details";

export const metadata: Metadata = {
  title: "Composer",
  description: "A composer's discography and compositions.",
};

export default function ComposerPage() {
  return (
    <Suspense fallback={<RouteLoading />}>
      <ComposerDetails />
    </Suspense>
  );
}
